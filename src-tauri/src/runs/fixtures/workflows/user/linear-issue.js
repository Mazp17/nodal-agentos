export const meta = {
  name: 'linear-issue',
  description:
    'Lleva una issue de Linear (con o sin sub-issues) hasta un PR o hasta Blocked, sincronizando el estado en Linear con la realidad del run',
  whenToUse:
    'Para resolver una issue de Linear de punta a punta. args: "ACME-8" o {issue, worktreeBase, baseRef, resumeBranch, extraWork}. Descubre sub-issues, respeta las relaciones blocking, verifica con la suite del repo, pasa por un code-reviewer sin permiso de escritura y termina siempre en PR+In Review o en Blocked con comentario. Cada run trabaja en su propio worktree y nunca hace checkout en el árbol compartido, así que se pueden disparar varios a la vez sobre el mismo repo. Con resumeBranch retoma una rama ya trabajada y va directo al gate. Nunca mergea.',
  phases: [
    { title: 'Leer', detail: 'issue, sub-issues, relaciones blocking y estados reales del team' },
    { title: 'Preparar', detail: 'descubrir los comandos de verificación y crear la rama de integración' },
    { title: 'Planificar', detail: 'un agente lee todas las sub-issues y decide qué corre en cada nivel' },
    { title: 'Implementar', detail: 'un agente por sub-issue, por niveles; al terminar cada una pasa a In Review' },
    { title: 'Integrar', detail: 'llevar cada rama hija a la rama de integración' },
    { title: 'Verificar', detail: 'la suite completa sobre el resultado integrado' },
    { title: 'Revisar', detail: 'code-reviewer sin Edit ni Write contra los criterios de aceptación' },
    { title: 'Corregir', detail: 'comentar los hallazgos en Linear y devolverlos al agente que implementó' },
    { title: 'Cerrar', detail: 'PR e In Review, o Blocked con comentario. Nunca merge.' },
  ],
}

// ------------------------------------------------------------------ entrada

const ISSUE = (typeof args === 'string' ? args : (args && (args.issue || args.id)) || '').trim()
if (!ISSUE) throw new Error('Falta el ID de la issue. Pasá args: "ACME-8" o {issue:"ACME-8"}.')

// El argumento puede llegar como "ACME-2", como URL de Linear, o con texto pegado detras.
