//! Config local de la app: mapeo team/proyecto de Linear → repo git y concurrencia.
//! Vive en `<app_config_dir>/config.json`.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const FILE_NAME: &str = "config.json";
pub const DEFAULT_CONCURRENCY: u32 = 3;
pub const MAX_CONCURRENCY: u32 = 16;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMapping {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub path: String,
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
    let find = |team: Option<&str>, project: Option<&str>| {
        config
            .repos
            .iter()
            .find(|m| m.team_id.as_deref() == team && m.project_id.as_deref() == project)
    };
    project_id
        .and_then(|pid| find(Some(team_id), Some(pid)).or_else(|| find(None, Some(pid))))
        .or_else(|| find(Some(team_id), None))
        .map(|m| m.path.clone())
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
        errors.push(format!("La concurrencia debe estar entre 1 y {MAX_CONCURRENCY}."));
    }

    for m in config.repos {
        let team_id = clean_id(&m.team_id);
        let project_id = clean_id(&m.project_id);
        let raw = m.path.trim();
        if team_id.is_none() && project_id.is_none() {
            errors.push(format!("El mapeo a «{raw}» no tiene team ni proyecto."));
            continue;
        }
        if !seen.insert((team_id.clone(), project_id.clone())) {
            errors.push(format!("Hay dos mapeos para el mismo team/proyecto (ruta «{raw}»)."));
            continue;
        }
        if raw.is_empty() {
            errors.push("Hay un mapeo con la ruta vacía.".into());
            continue;
        }
        let path = expand_home(raw);
        if !path.is_absolute() {
            errors.push(format!("«{raw}» no es una ruta absoluta."));
        } else if !path.is_dir() {
            errors.push(format!("«{raw}» no existe o no es una carpeta."));
        } else if !is_git_repo(&path) {
            errors.push(format!("«{raw}» no es la raíz de un repo git (falta .git)."));
        } else {
            repos.push(RepoMapping {
                team_id,
                project_id,
                path: path.to_string_lossy().into_owned(),
            });
        }
    }

    if errors.is_empty() {
        Ok(AppConfig { repos, concurrency: config.concurrency })
    } else {
        Err(errors.join("\n"))
    }
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|d| d.join(FILE_NAME))
        .map_err(|e| format!("No se encontró la carpeta de configuración: {e}"))
}

pub fn load_from(path: &Path) -> Result<AppConfig, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s)
            .map_err(|e| format!("{} está corrupto: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(e) => Err(format!("No se pudo leer {}: {e}", path.display())),
    }
}

/// Escritura atómica: archivo temporal + rename.
fn save_to(path: &Path, config: &AppConfig) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("No se pudo crear {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, json).map_err(|e| format!("No se pudo escribir la config: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("No se pudo guardar la config: {e}"))
}

/// Las operaciones de archivo son bloqueantes: corren fuera del runtime async.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("Fallo interno: {e}"))?
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

#[cfg(test)]
mod tests {
    use super::*;

    fn map(team: Option<&str>, project: Option<&str>, path: &str) -> RepoMapping {
        RepoMapping {
            team_id: team.map(String::from),
            project_id: project.map(String::from),
            path: path.into(),
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
        assert!(err.contains("falta .git"), "{err}");
        assert!(err.contains("no existe"), "{err}");
        assert!(err.contains("absoluta"), "{err}");
        assert!(err.contains("no tiene team ni proyecto"), "{err}");
        assert!(err.contains("mismo team/proyecto"), "{err}");
        assert!(err.contains("concurrencia"), "{err}");
        std::fs::remove_dir_all(plain).unwrap();
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = temp_dir("cfg");
        let path = dir.join("nested").join(FILE_NAME);
        assert_eq!(load_from(&path).unwrap(), AppConfig::default());
        save_to(&path, &fixture()).unwrap();
        assert_eq!(load_from(&path).unwrap(), fixture());
        std::fs::write(&path, "{nope").unwrap();
        assert!(load_from(&path).unwrap_err().contains("corrupto"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
