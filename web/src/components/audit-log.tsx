import { useQuery } from '@tanstack/react-query'
import { cn } from '#/lib/utils'
import { api } from '#/lib/api'

export function AuditLog() {
  const audit = useQuery({ queryKey: ['audit'], queryFn: api.audit, refetchInterval: 2000 })
  const entries = audit.data?.entries ?? []

  if (entries.length === 0) {
    return <p className="text-muted-foreground px-6 pb-6 text-sm">Aucune requête pour l’instant.</p>
  }

  return (
    <div className="max-h-96 overflow-y-auto px-6 pb-6">
      <table className="w-full font-mono text-xs">
        <tbody>
          {entries.map((e, i) => (
            <tr key={`${e.at}-${i}`} className="border-b last:border-0">
              <td className="text-muted-foreground py-1.5 pr-4 whitespace-nowrap">
                {new Date(e.at).toLocaleTimeString('fr-FR')}
              </td>
              <td className="py-1.5 pr-4 whitespace-nowrap">{e.route}</td>
              <td className="py-1.5 pr-4">{e.method}</td>
              <td className="max-w-md truncate py-1.5 pr-4" title={e.path}>
                {e.path}
              </td>
              <td
                className={cn(
                  'py-1.5 pr-4 font-semibold tabular-nums',
                  !e.forwarded
                    ? 'text-destructive-foreground'
                    : e.status < 400
                      ? 'text-success'
                      : 'text-warning',
                )}
              >
                {e.status}
              </td>
              <td className="text-muted-foreground py-1.5 font-sans">{e.note}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
