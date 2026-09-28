export const meta = {
  name: 'demo-board',
  description: 'Workflow de juguete para probar el board: arma una mini guía sobre un tema inocuo, sin tocar repos ni Linear',
  whenToUse:
    'Solo para probar la herramienta de board. args: "volcanes" o {tema, outDir}. Escribe archivos en outDir (default ./demo-out, relativo al cwd). No usa Linear ni git.',
  phases: [
    { title: 'Idear', detail: 'un agente propone 4 subtemas' },
    { title: 'Escribir', detail: 'un agente por subtema escribe un archivo markdown' },
    { title: 'Revisar', detail: 'un agente por archivo lo revisa, en pipeline con Escribir' },
    { title: 'Cerrar', detail: 'un agente arma el índice y corre un comando de shell' },
  ],
}

const TEMA = String((typeof args === 'string' ? args : args && args.tema) || 'volcanes').trim()
const OUT = String((args && typeof args === 'object' && args.outDir) || './demo-out').trim()
// OUT termina en prompts y en un comando de shell: solo rutas relativas simples, sin salir del cwd.
if (!/^[\w.\/-]+$/.test(OUT) || OUT.startsWith('/') || OUT.split('/').includes('..'))
  throw new Error(`outDir inválido: "${OUT}". Usá una ruta relativa sin ".." ni caracteres especiales.`)

// Modelo barato y effort bajo: el objetivo es ejercitar la herramienta, no la calidad del texto.
const CHEAP = { model: 'claude-haiku-4-5-20251001', effort: 'low' }

const IDEAS = {
  type: 'object',
  properties: {
    subtemas: {
      type: 'array',
      minItems: 4,
      maxItems: 4,
      items: {
        type: 'object',
        properties: {
          slug: { type: 'string', description: 'kebab-case, sin espacios ni barras' },
          titulo: { type: 'string' },
        },
        required: ['slug', 'titulo'],
      },
    },
  },
  required: ['subtemas'],
}

const ESCRITO = {
  type: 'object',
  properties: { path: { type: 'string' }, palabras: { type: 'number' } },
  required: ['path', 'palabras'],
}

const REVISION = {
  type: 'object',
  properties: { path: { type: 'string' }, ok: { type: 'boolean' }, nota: { type: 'string' } },
  required: ['path', 'ok', 'nota'],
}

phase('Idear')
const ideas = await agent(
  `Proponé exactamente 4 subtemas cortos para una mini guía de divulgación sobre "${TEMA}". Solo pensá, no uses herramientas.`,
  { ...CHEAP, label: `idear:${TEMA}`, schema: IDEAS },
)
if (!ideas) throw new Error('El agente de Idear no devolvió nada.')
// El slug lo inventa el LLM y va a una ruta de archivo: se sanea acá, no se confía en el schema.
const subtemas = ideas.subtemas.map((s) => ({ ...s, slug: s.slug.toLowerCase().replace(/[^a-z0-9-]/g, '-') }))
log(`Subtemas: ${subtemas.map((s) => s.titulo).join(' · ')}`)

// Pipeline sin barrera: un subtema puede estar en Revisar mientras otro sigue en Escribir,
// que es justo lo que el board tiene que saber mostrar.
const revisados = await pipeline(
  subtemas,
  (s, _orig, i) =>
    agent(
      `Escribí un markdown de 120 a 180 palabras sobre "${s.titulo}" (tema general: ${TEMA}).
Guardalo en ${OUT}/${String(i + 1).padStart(2, '0')}-${s.slug}.md, creando el directorio con mkdir -p si hace falta.
Antes de escribir, corré \`sleep ${10 + i * 8}\` en Bash: es a propósito, para que el run dure lo suficiente como para observarlo.
Devolvé la ruta y la cantidad aproximada de palabras.`,
      { ...CHEAP, label: `escribir:${s.slug}`, phase: 'Escribir', schema: ESCRITO },
    ),
  (e, s) =>
    e &&
    agent(
      `Leé ${e.path} y revisalo: ¿está en español, entre 120 y 180 palabras y sobre "${s.titulo}"? No lo modifiques.`,
      { ...CHEAP, label: `revisar:${s.slug}`, phase: 'Revisar', schema: REVISION },
    ).then((r) => r && { ...r, path: e.path }),
)

const ok = revisados.filter(Boolean)
if (ok.length < subtemas.length) log(`${subtemas.length - ok.length} subtema(s) se cayeron en el camino.`)
if (!ok.length) return { tema: TEMA, outDir: OUT, status: 'red', revisiones: [], cierre: null }

phase('Cerrar')
const cierre = await agent(
  `Creá ${OUT}/README.md con un título sobre "${TEMA}" y una lista enlazando estos archivos: ${ok.map((r) => r.path).join(', ')}.
Después corré \`ls -la ${OUT} && wc -w ${OUT}/*.md\` y devolvé la salida tal cual.`,
  { ...CHEAP, label: 'cerrar:indice' },
)

return {
  tema: TEMA,
  outDir: OUT,
  status: ok.length === subtemas.length && ok.every((r) => r.ok) ? 'green' : 'yellow',
  revisiones: ok,
  cierre,
}
