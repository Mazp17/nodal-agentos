// Fixture sintético con la forma del meta de un workflow real (no es el workflow completo).
export const meta = {
  name: 'linear-issue',
  description:
    'Lleva una issue de Linear hasta un PR o hasta Blocked, sincronizando el estado con la realidad del run',
  whenToUse:
    'Para resolver una issue de Linear de punta a punta. args: "ACME-8" o {issue, baseRef}. Pasa por un code-reviewer y termina en PR+In Review o en Blocked con comentario. Nunca mergea.',
  phases: [
    { title: 'Leer', detail: 'issue, sub-issues y estados del team' },
    { title: 'Implementar', detail: 'un agente por sub-issue' },
    { title: 'Revisar', detail: 'code-reviewer sin Edit ni Write' },
    { title: 'Cerrar', detail: 'PR e In Review, o Blocked con comentario. Nunca merge.' },
  ],
}

const ISSUE = (typeof args === 'string' ? args : (args && args.issue) || '').trim()
if (!ISSUE) throw new Error('Falta el ID de la issue. Pasá args: "ACME-8" o {issue:"ACME-8"}.')
