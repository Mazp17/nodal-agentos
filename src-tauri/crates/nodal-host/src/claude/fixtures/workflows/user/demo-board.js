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

// Sonnet y effort bajo: el objetivo es ejercitar la herramienta, no la calidad del texto.
