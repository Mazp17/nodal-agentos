//! Config local de la app: mapeo team/proyecto de Linear → repo git y concurrencia.
//! Vive en `<app_config_dir>/config.json`.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::runs::claude_bin;
use crate::runs::options;
use crate::runs::types::LaunchOptions;

const FILE_NAME: &str = "config.json";
pub const DEFAULT_CONCURRENCY: u32 = 3;
pub const MAX_CONCURRENCY: u32 = 16;

/// Cómo termina un workflow que lo respete (p. ej. `plan-task`): PR abierto o solo rama.
pub const FINISH_MODES: [&str; 2] = ["pr", "branch"];

/// Todos los campos salvo `path` son opcionales: un config.json viejo sigue cargando.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMapping {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub path: String,
    /// `--model` para los runs de este repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `--effort`: low | medium | high | xhigh | max.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// `--permission-mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// "pr" (default) | "branch". No se pasa a `claude`: lo leen los workflows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
}

impl RepoMapping {
    pub fn launch_options(&self) -> LaunchOptions {
        LaunchOptions {
            model: self.model.clone(),
            effort: self.effort.clone(),
            permission_mode: self.permission_mode.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default)]
    pub repos: Vec<RepoMapping>,
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
}

fn default_concurrency() -> u32 {
    DEFAULT_CONCURRENCY
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { repos: Vec::new(), concurrency: DEFAULT_CONCURRENCY }
    }
}

/// Repo para una issue. Gana el mapeo más específico, sin importar el orden en la lista:
/// 1. team + proyecto exactos, 2. sólo proyecto, 3. sólo team.
pub fn resolve(config: &AppConfig, team_id: &str, project_id: Option<&str>) -> Option<String> {
    resolve_mapping(config, team_id, project_id).map(|m| m.path.clone())
}

/// Como `resolve`, pero devuelve el mapeo entero (model, effort, ...).
pub fn resolve_mapping<'a>(config: &'a AppConfig, team_id: &str, project_id: Option<&str>) -> Option<&'a RepoMapping> {
    let find = |team: Option<&str>, project: Option<&str>| {
        config
            .repos
            .iter()
            .find(|m| m.team_id.as_deref() == team && m.project_id.as_deref() == project)
    };
    project_id
        .and_then(|pid| find(Some(team_id), Some(pid)).or_else(|| find(None, Some(pid))))
        .or_else(|| find(Some(team_id), None))
}

fn normalize_path(p: &str) -> PathBuf {
    let p = expand_home(p.trim());
    let s = p.to_string_lossy();
    let trimmed = s.trim_end_matches('/');
    PathBuf::from(if trimmed.is_empty() { "/" } else { trimmed })
}

/// Primer mapeo cuyo repo es `path` (para lanzar tareas que no vienen de Linear).
/// Si varios mapeos apuntan al mismo repo, gana el primero de la lista.
pub fn find_by_path<'a>(config: &'a AppConfig, path: &str) -> Option<&'a RepoMapping> {
    let want = normalize_path(path);
    config.repos.iter().find(|m| normalize_path(&m.path) == want)
}

fn clean_id(id: &Option<String>) -> Option<String> {
    id.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ if path == "~" => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default(),
        _ => PathBuf::from(path),
    }
}

/// Raíz de un repo git: tiene `.git` (directorio, o archivo en worktrees/submódulos).
fn is_git_repo(p: &Path) -> bool {
    p.join(".git").exists()
}

/// Normaliza (trim, `~`) y valida. Devuelve todos los problemas juntos.
pub fn validate(config: AppConfig) -> Result<AppConfig, String> {
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    let mut repos = Vec::with_capacity(config.repos.len());

    if config.concurrency == 0 || config.concurrency > MAX_CONCURRENCY {
        errors.push(format!("Parallel runs must be between 1 and {MAX_CONCURRENCY}."));
    }

    for m in config.repos {
        let team_id = clean_id(&m.team_id);
        let project_id = clean_id(&m.project_id);
        let raw = m.path.trim();
        if team_id.is_none() && project_id.is_none() {
            errors.push(format!("The mapping to \"{raw}\" has no team or project."));
            continue;
        }
        if !seen.insert((team_id.clone(), project_id.clone())) {
            errors.push(format!("There are two mappings for the same team/project (path \"{raw}\")."));
            continue;
        }
        if raw.is_empty() {
            errors.push("A mapping has an empty path.".into());
            continue;
        }
        let opts = match options::normalize(&m.launch_options()) {
            Ok(o) => o,
            Err(errs) => {
                errors.extend(errs.into_iter().map(|e| format!("\"{raw}\": {e}")));
                continue;
            }
        };
        let finish = clean_id(&m.finish);
        if let Some(f) = &finish {
            if !FINISH_MODES.contains(&f.as_str()) {
                errors.push(format!("\"{raw}\": invalid finish \"{f}\": use pr or branch."));
                continue;
            }
        }
        let path = expand_home(raw);
        if !path.is_absolute() {
            errors.push(format!("\"{raw}\" is not an absolute path."));
        } else if !path.is_dir() {
            errors.push(format!("\"{raw}\" doesn't exist or isn't a folder."));
        } else if !is_git_repo(&path) {
            errors.push(format!("\"{raw}\" is not the root of a git repo (no .git)."));
        } else {
            repos.push(RepoMapping {
                team_id,
                project_id,
                path: path.to_string_lossy().into_owned(),
                model: opts.model,
                effort: opts.effort,
                permission_mode: opts.permission_mode,
                finish,
            });
        }
    }

    if errors.is_empty() {
        Ok(AppConfig { repos, concurrency: config.concurrency })
    } else {
        Err(errors.join("\n"))
    }
}

pub(crate) fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|d| d.join(FILE_NAME))
        .map_err(|e| format!("Couldn't find the app config folder: {e}"))
}

pub fn load_from(path: &Path) -> Result<AppConfig, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{} is corrupt: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(e) => Err(format!("Couldn't read {}: {e}", path.display())),
    }
}

/// Escritura atómica: archivo temporal + rename.
fn save_to(path: &Path, config: &AppConfig) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, json).map_err(|e| format!("Couldn't write the config: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't save the config: {e}"))
}

/// Las operaciones de archivo son bloqueantes: corren fuera del runtime async.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Internal error: {e}"))?
}

#[tauri::command]
pub async fn get_config(app: AppHandle) -> Result<AppConfig, String> {
    let path = config_path(&app)?;
    blocking(move || load_from(&path)).await
}

/// Valida y guarda; devuelve la config normalizada (rutas expandidas, ids limpios).
#[tauri::command]
pub async fn save_config(app: AppHandle, config: AppConfig) -> Result<AppConfig, String> {
    let path = config_path(&app)?;
    blocking(move || {
        let config = validate(config)?;
        save_to(&path, &config)?;
        Ok(config)
    })
    .await
}

#[tauri::command]
pub async fn resolve_repo(
    app: AppHandle,
    team_id: String,
    project_id: Option<String>,
) -> Result<Option<String>, String> {
    let path = config_path(&app)?;
    let config = blocking(move || load_from(&path)).await?;
    Ok(resolve(&config, &team_id, project_id.as_deref()))
}

/// Mapeo de un repo por su ruta (model, effort, permission mode, finish).
#[tauri::command]
pub async fn resolve_repo_config(app: AppHandle, path: String) -> Result<Option<RepoMapping>, String> {
    let cfg_path = config_path(&app)?;
    let config = blocking(move || load_from(&cfg_path)).await?;
    Ok(find_by_path(&config, &path).cloned())
}

const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Raíz del repo git que contiene `path` (`git rev-parse --show-toplevel`), o `None` si
/// no está dentro de un repo. Para ofrecer la raíz cuando eligen una subcarpeta.
#[tauri::command]
pub async fn resolve_git_root(path: String) -> Result<Option<String>, String> {
    let dir = expand_home(path.trim());
    if !dir.is_absolute() {
        return Err(format!("The folder must be an absolute path: {path}"));
    }
    let is_dir = {
        let dir = dir.clone();
        blocking(move || Ok(dir.is_dir())).await?
    };
    if !is_dir {
        return Err(format!("The folder doesn't exist: {path}"));
    }
    let mut cmd = claude_bin::tool_command("git").ok_or("Couldn't find `git` in PATH.")?;
    cmd.arg("-C").arg(&dir).args(["rev-parse", "--show-toplevel"]);
    let out = claude_bin::output_with_timeout(cmd, GIT_TIMEOUT, "`git rev-parse`").await?;
    if !out.status.success() {
        // Fuera de un repo git: no es un error, simplemente no hay raíz.
        return Ok(None);
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!root.is_empty()).then_some(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(team: Option<&str>, project: Option<&str>, path: &str) -> RepoMapping {
        RepoMapping {
            team_id: team.map(String::from),
            project_id: project.map(String::from),
            path: path.into(),
            ..RepoMapping::default()
        }
    }

    fn fixture() -> AppConfig {
        AppConfig {
            repos: vec![
                map(Some("t1"), None, "/repos/team1"),
                map(None, Some("p1"), "/repos/project1"),
                map(Some("t2"), Some("p2"), "/repos/t2-project2"),
                map(Some("t2"), None, "/repos/team2"),
            ],
            concurrency: 3,
        }
    }

    #[test]
    fn project_mapping_beats_team_mapping() {
        let c = fixture();
        assert_eq!(resolve(&c, "t1", Some("p1")).as_deref(), Some("/repos/project1"));
        assert_eq!(resolve(&c, "t2", Some("p2")).as_deref(), Some("/repos/t2-project2"));
    }

    #[test]
    fn falls_back_to_team_mapping() {
        let c = fixture();
        assert_eq!(resolve(&c, "t1", None).as_deref(), Some("/repos/team1"));
        assert_eq!(resolve(&c, "t1", Some("otro")).as_deref(), Some("/repos/team1"));
        // Mapeo de proyecto atado a t2 no aplica a issues de t1 en ese proyecto.
        assert_eq!(resolve(&c, "t1", Some("p2")).as_deref(), Some("/repos/team1"));
    }

    #[test]
    fn exact_team_project_beats_project_only_regardless_of_order() {
        let c = AppConfig {
            repos: vec![
                map(None, Some("p1"), "/repos/project1"),
                map(Some("t1"), Some("p1"), "/repos/t1-project1"),
            ],
            concurrency: 3,
        };
        assert_eq!(resolve(&c, "t1", Some("p1")).as_deref(), Some("/repos/t1-project1"));
        assert_eq!(resolve(&c, "t2", Some("p1")).as_deref(), Some("/repos/project1"));
    }

    #[test]
    fn project_only_mapping_applies_to_any_team() {
        let c = fixture();
        assert_eq!(resolve(&c, "t9", Some("p1")).as_deref(), Some("/repos/project1"));
    }

    #[test]
    fn unmapped_returns_none() {
        let c = fixture();
        assert_eq!(resolve(&c, "t9", None), None);
        assert_eq!(resolve(&AppConfig::default(), "t1", Some("p1")), None);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let c: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c, AppConfig::default());
        let c: AppConfig =
            serde_json::from_str(r#"{"repos":[{"teamId":"t1","path":"/x"}]}"#).unwrap();
        assert_eq!(c.concurrency, DEFAULT_CONCURRENCY);
        assert_eq!(c.repos[0].project_id, None);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agent-desk-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn validate_accepts_git_repo_and_normalizes() {
        let repo = temp_dir("repo");
        std::fs::create_dir(repo.join(".git")).unwrap();
        let input = AppConfig {
            repos: vec![map(Some(" t1 "), Some(""), &format!("  {}  ", repo.display()))],
            concurrency: 2,
        };
        let out = validate(input).unwrap();
        assert_eq!(out.repos, vec![map(Some("t1"), None, &repo.to_string_lossy())]);
        std::fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn validate_rejects_bad_entries() {
        let plain = temp_dir("plain");
        let input = AppConfig {
            repos: vec![
                map(Some("t1"), None, &plain.to_string_lossy()),
                map(Some("t2"), None, "/no/existe/seguro"),
                map(Some("t3"), None, "relativa/x"),
                map(None, None, "/tmp"),
                map(Some("t1"), None, "/otra"),
            ],
            concurrency: 0,
        };
        let err = validate(input).unwrap_err();
        assert!(err.contains("no .git"), "{err}");
        assert!(err.contains("doesn't exist"), "{err}");
        assert!(err.contains("absolute"), "{err}");
        assert!(err.contains("no team or project"), "{err}");
        assert!(err.contains("same team/project"), "{err}");
        assert!(err.contains("Parallel runs"), "{err}");
        std::fs::remove_dir_all(plain).unwrap();
    }

    #[test]
    fn validate_checks_and_keeps_launch_options() {
        let repo = temp_dir("repo-opts");
        std::fs::create_dir(repo.join(".git")).unwrap();
        let p = repo.to_string_lossy().into_owned();
        let good = RepoMapping {
            model: Some(" opus ".into()),
            effort: Some("xhigh".into()),
            permission_mode: Some("acceptEdits".into()),
            finish: Some("branch".into()),
            ..map(Some("t1"), None, &p)
        };
        let out = validate(AppConfig { repos: vec![good], concurrency: 2 }).unwrap();
        let m = &out.repos[0];
        assert_eq!(
            (m.model.as_deref(), m.effort.as_deref(), m.permission_mode.as_deref(), m.finish.as_deref()),
            (Some("opus"), Some("xhigh"), Some("acceptEdits"), Some("branch"))
        );
        let bad = vec![
            RepoMapping { effort: Some("ultra".into()), ..map(Some("t1"), None, &p) },
            RepoMapping { permission_mode: Some("yolo".into()), ..map(Some("t2"), None, &p) },
            RepoMapping { model: Some("--dangerously".into()), ..map(Some("t3"), None, &p) },
            RepoMapping { finish: Some("merge".into()), ..map(Some("t4"), None, &p) },
        ];
        let err = validate(AppConfig { repos: bad, concurrency: 2 }).unwrap_err();
        assert_eq!(err.lines().count(), 4, "{err}");
        assert!(err.contains("Invalid effort") && err.contains("Invalid permission mode"), "{err}");
        assert!(err.contains("Invalid model") && err.contains("invalid finish"), "{err}");
        std::fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn old_config_without_options_still_loads() {
        let c: AppConfig =
            serde_json::from_str(r#"{"repos":[{"teamId":"t1","path":"/x"}],"concurrency":2}"#).unwrap();
        assert_eq!(c.repos[0].launch_options(), LaunchOptions::default());
        assert_eq!(c.repos[0].finish, None);
        // Y al guardar no aparecen campos nuevos vacíos.
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, r#"{"repos":[{"teamId":"t1","path":"/x"}],"concurrency":2}"#);
    }

    #[test]
    fn finds_mapping_by_path() {
        let mut c = fixture();
        c.repos[1].model = Some("opus".into());
        assert_eq!(find_by_path(&c, "/repos/project1/").and_then(|m| m.model.as_deref()), Some("opus"));
        assert_eq!(find_by_path(&c, " /repos/team2 ").map(|m| m.team_id.as_deref()), Some(Some("t2")));
        assert!(find_by_path(&c, "/repos/otro").is_none());
        assert_eq!(
            resolve_mapping(&c, "t1", Some("p1")).and_then(|m| m.model.as_deref()),
            Some("opus")
        );
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = temp_dir("cfg");
        let path = dir.join("nested").join(FILE_NAME);
        assert_eq!(load_from(&path).unwrap(), AppConfig::default());
        save_to(&path, &fixture()).unwrap();
        assert_eq!(load_from(&path).unwrap(), fixture());
        std::fs::write(&path, "{nope").unwrap();
        assert!(load_from(&path).unwrap_err().contains("corrupt"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn git_root_of_subfolder() {
        // Repo temporal propio (no depende de que el crate esté en un checkout git).
        let repo = temp_dir("git-root").canonicalize().unwrap();
        let ok = std::process::Command::new("git").arg("init").arg("-q").arg(&repo).status();
        if !ok.is_ok_and(|s| s.success()) {
            eprintln!("git no disponible: se saltea");
            return;
        }
        std::fs::create_dir_all(repo.join("a/b")).unwrap();
        let sub = repo.join("a/b").to_string_lossy().into_owned();
        let root = tauri::async_runtime::block_on(resolve_git_root(sub)).unwrap();
        assert_eq!(root.as_deref(), Some(repo.to_string_lossy().as_ref()));
        std::fs::remove_dir_all(repo).unwrap();
        let tmp = temp_dir("not-git");
        assert_eq!(tauri::async_runtime::block_on(resolve_git_root(tmp.to_string_lossy().into_owned())).unwrap(), None);
        assert!(tauri::async_runtime::block_on(resolve_git_root("relativa".into())).is_err());
        std::fs::remove_dir_all(tmp).unwrap();
    }
}
