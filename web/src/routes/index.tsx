import { useState } from 'react'
import { createFileRoute } from '@tanstack/react-router'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Copy, Fingerprint, KeyRound, Loader2, Plus, RotateCw, TriangleAlert } from 'lucide-react'
import { toast } from 'sonner'
import { AuditLog } from '#/components/audit-log'
import { RouteFormDialog } from '#/components/route-form-dialog'
import { RoutesTable } from '#/components/routes-table'
import { ThemeToggle } from '#/components/theme-toggle'
import { Button } from '#/components/ui/button'
import { Card, CardAction, CardContent, CardDescription, CardHeader, CardTitle } from '#/components/ui/card'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '#/components/ui/select'
import { api, groupsOf } from '#/lib/api'
import type { RouteView } from '#/lib/api'

export const Route = createFileRoute('/')({ component: Dashboard })

/** Group filter value showing every route. */
const ALL_GROUPS = '*'

function Dashboard() {
  const status = useQuery({ queryKey: ['status'], queryFn: api.status, refetchInterval: 2000 })
  const routes = useQuery({ queryKey: ['routes'], queryFn: api.routes, refetchInterval: 5000 })
  const [editor, setEditor] = useState<{ open: boolean; route: RouteView | null }>({
    open: false,
    route: null,
  })
  const [groupFilter, setGroupFilter] = useState<string>(ALL_GROUPS)

  const proxy = status.data?.proxy
  const groups = groupsOf(routes.data ?? [])
  // A filter on a group that no longer exists shows everything.
  const shownGroup = groups.includes(groupFilter) ? groupFilter : ALL_GROUPS
  const shownRoutes = (routes.data ?? []).filter(
    (r) => shownGroup === ALL_GROUPS || r.group === shownGroup,
  )

  return (
    <div className="mx-auto flex min-h-screen max-w-6xl flex-col gap-6 px-4 py-8 sm:px-6">
      <header className="flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <div className="bg-primary text-primary-foreground flex size-10 items-center justify-center rounded-lg">
            <KeyRound className="size-5" />
          </div>
          <div>
            <h1 className="text-xl font-semibold tracking-tight">Token Proxy</h1>
            <ProxyLine proxy={proxy} unreachable={status.isError} />
          </div>
        </div>
        <div className="flex gap-2">
          <Button onClick={() => setEditor({ open: true, route: null })}>
            <Plus /> Ajouter une route
          </Button>
          <ThemeToggle />
        </div>
      </header>

      {status.data?.vault_error && <VaultErrorBanner error={status.data.vault_error} />}
      {status.data?.approval_pending && (
        <Banner tone="info">
          <Fingerprint className="size-4" /> Une demande Touch ID est en attente. N’approuve que ce que
          tu as demandé toi-même.
        </Banner>
      )}

      <Card>
        <CardHeader>
          <CardTitle>Routes</CardTitle>
          <CardDescription>
            L’IA appelle /&lt;groupe&gt;/&lt;route&gt;/… : le groupe dit à qui appartient le compte.
            Chaque route démarre désactivée. Activer, ajouter, modifier ou supprimer une route
            demande Touch ID ; la désactiver, jamais.
          </CardDescription>
          {groups.length > 1 && (
            <CardAction>
              <Select value={shownGroup} onValueChange={setGroupFilter}>
                <SelectTrigger size="sm" className="w-44">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={ALL_GROUPS}>Tous les groupes</SelectItem>
                  {groups.map((g) => (
                    <SelectItem key={g} value={g}>
                      {g}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </CardAction>
          )}
        </CardHeader>
        <CardContent>
          {routes.data && routes.data.length > 0 ? (
            <RoutesTable
              routes={shownRoutes}
              fetchedAt={routes.dataUpdatedAt}
              onEdit={(route) => setEditor({ open: true, route })}
            />
          ) : (
            <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed py-12 text-center">
              <p className="text-muted-foreground text-sm">
                {routes.isLoading ? 'Chargement…' : 'Aucune route pour l’instant. Enregistre un premier token pour commencer.'}
              </p>
              {!routes.isLoading && (
                <Button variant="outline" onClick={() => setEditor({ open: true, route: null })}>
                  <Plus /> Ajouter une route
                </Button>
              )}
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Requêtes récentes</CardTitle>
          <CardDescription>
            Depuis le démarrage du service. Les 30 derniers jours sont gardés dans ~/Library/Logs/token-proxy/audit.log
          </CardDescription>
        </CardHeader>
        <AuditLog />
      </Card>

      <RouteFormDialog
        open={editor.open}
        route={editor.route}
        groups={groups}
        onOpenChange={(open) => setEditor((e) => ({ ...e, open }))}
      />
    </div>
  )
}

function ProxyLine({
  proxy,
  unreachable,
}: {
  proxy: { state: string; url?: string; error?: string } | undefined
  unreachable: boolean
}) {
  if (unreachable) {
    return <p className="text-destructive-foreground text-sm">API d’administration injoignable</p>
  }
  if (proxy?.state === 'listening' && proxy.url) {
    const url = proxy.url
    return (
      <p className="text-muted-foreground flex items-center gap-2 text-sm">
        <span className="bg-success size-2 rounded-full" />
        Proxy sur <span className="font-mono">{url}</span>
        <button
          type="button"
          className="hover:text-foreground"
          aria-label="Copier l’URL du proxy"
          onClick={() => {
            navigator.clipboard.writeText(url)
            toast.success('URL du proxy copiée')
          }}
        >
          <Copy className="size-3.5" />
        </button>
      </p>
    )
  }
  if (proxy?.state === 'failed') {
    return <p className="text-destructive-foreground text-sm">Proxy arrêté : {proxy.error}</p>
  }
  return <p className="text-muted-foreground text-sm">Démarrage…</p>
}

function VaultErrorBanner({ error }: { error: string }) {
  const queryClient = useQueryClient()
  const reload = useMutation({
    mutationFn: api.reloadVault,
    onSuccess: () => {
      toast.success('Coffre chargé, les routes sont de retour')
      queryClient.invalidateQueries()
    },
    onError: (e) => toast.error(e.message),
  })

  return (
    <Banner
      tone="error"
      action={
        <Button size="sm" variant="outline" onClick={() => reload.mutate()} disabled={reload.isPending}>
          {reload.isPending ? <Loader2 className="animate-spin" /> : <RotateCw />}
          {reload.isPending ? 'En attente du Trousseau…' : 'Réessayer'}
        </Button>
      }
    >
      <span>
        Le coffre du Trousseau n’a pas pu être lu : aucune route n’est disponible ({error}).
        Clique sur « Réessayer », puis sur « Toujours autoriser » dans la demande de macOS.
      </span>
    </Banner>
  )
}

function Banner({
  tone,
  action,
  children,
}: {
  tone: 'error' | 'info'
  action?: React.ReactNode
  children: React.ReactNode
}) {
  const style =
    tone === 'error'
      ? 'border-destructive/50 text-destructive-foreground'
      : 'border-warning/50 text-foreground'
  return (
    <div className={`flex items-center gap-2 rounded-lg border px-4 py-3 text-sm ${style}`}>
      {tone === 'error' && <TriangleAlert className="size-4 shrink-0" />}
      <div className="flex flex-1 items-center gap-2">{children}</div>
      {action}
    </div>
  )
}
