import { Fragment, useEffect, useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { ChevronDown, Fingerprint, MoreHorizontal, Pencil, Power, Trash2 } from 'lucide-react'
import { toast } from 'sonner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '#/components/ui/alert-dialog'
import { Badge } from '#/components/ui/badge'
import { Button } from '#/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '#/components/ui/dropdown-menu'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '#/components/ui/table'
import { api, groupsOf, hostOf, routeId } from '#/lib/api'
import type { RouteView } from '#/lib/api'

const DURATIONS: { label: string; minutes: number | null }[] = [
  { label: '15 minutes', minutes: 15 },
  { label: '1 heure', minutes: 60 },
  { label: '4 heures', minutes: 240 },
  { label: 'Jusqu’à désactivation', minutes: null },
]

interface Props {
  routes: RouteView[]
  /** When the route list was fetched, to count expirations down locally. */
  fetchedAt: number
  onEdit: (route: RouteView) => void
}

export function RoutesTable({ routes, fetchedAt, onEdit }: Props) {
  const [toDelete, setToDelete] = useState<RouteView | null>(null)
  const queryClient = useQueryClient()
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['routes'] })

  const remove = useMutation({
    mutationFn: (route: RouteView) => api.deleteRoute(route),
    onSuccess: (_, route) => {
      toast.success(`Route « ${routeId(route)} » supprimée`)
      setToDelete(null)
      refresh()
    },
    onError: (e) => toast.error(e.message),
  })

  return (
    <>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Route</TableHead>
            <TableHead>Service</TableHead>
            <TableHead>État</TableHead>
            <TableHead className="text-right">Actions</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {groupsOf(routes).map((group) => {
            const members = routes
              .filter((r) => r.group === group)
              .sort((a, b) => a.name.localeCompare(b.name))
            return (
              <Fragment key={group}>
                <TableRow className="bg-muted/50 hover:bg-muted/50">
                  <TableCell colSpan={4} className="py-1.5">
                    <span className="font-mono text-xs font-semibold">{group}</span>
                    <span className="text-muted-foreground ml-2 text-xs">
                      {members.length} route{members.length > 1 ? 's' : ''}
                    </span>
                  </TableCell>
                </TableRow>
                {members.map((route) => (
                  <TableRow key={routeId(route)}>
                    <TableCell className="align-top">
                      <div className="flex items-center gap-2 font-mono">
                        <span>
                          <span className="text-muted-foreground">{route.group}/</span>
                          <span className="font-medium">{route.name}</span>
                        </span>
                      </div>
                      {route.description && (
                        <p className="text-muted-foreground mt-1 max-w-xs text-xs whitespace-normal">
                          {route.description}
                        </p>
                      )}
                    </TableCell>
                    <TableCell className="align-top font-mono text-xs">{hostOf(route.upstream)}</TableCell>
                    <TableCell className="align-top">
                      <StatusBadge route={route} fetchedAt={fetchedAt} />
                    </TableCell>
                    <TableCell className="text-right align-top">
                      <div className="flex justify-end gap-2">
                        {route.enabled ? <DisableButton route={route} /> : <EnableMenu route={route} />}
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button variant="ghost" size="icon-sm" aria-label="Plus d’actions">
                              <MoreHorizontal />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onSelect={() => onEdit(route)}>
                              <Pencil /> Modifier
                            </DropdownMenuItem>
                            <DropdownMenuItem variant="destructive" onSelect={() => setToDelete(route)}>
                              <Trash2 /> Supprimer
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </Fragment>
            )
          })}
        </TableBody>
      </Table>

      <AlertDialog open={toDelete !== null} onOpenChange={(o) => !o && !remove.isPending && setToDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Supprimer « {toDelete && routeId(toDelete)} » ?</AlertDialogTitle>
            <AlertDialogDescription>
              La route et son token sont retirés du Trousseau. Cette action est irréversible.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={remove.isPending}>Annuler</AlertDialogCancel>
            <AlertDialogAction
              onClick={(e) => {
                e.preventDefault()
                if (toDelete) remove.mutate(toDelete)
              }}
              disabled={remove.isPending}
              className="bg-destructive hover:bg-destructive/90 text-white"
            >
              <Fingerprint />
              {remove.isPending ? 'En attente de Touch ID…' : 'Supprimer'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

function StatusBadge({ route, fetchedAt }: { route: RouteView; fetchedAt: number }) {
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    if (route.expires_in_seconds === null) return
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [route.expires_in_seconds])

  if (!route.enabled) {
    return (
      <Badge variant="secondary" className="text-muted-foreground">
        Désactivée
      </Badge>
    )
  }
  if (route.expires_in_seconds === null) {
    return <Badge className="bg-success text-white">Activée</Badge>
  }
  const left = Math.max(0, route.expires_in_seconds - Math.floor((now - fetchedAt) / 1000))
  const h = Math.floor(left / 3600)
  const m = Math.floor((left % 3600) / 60)
  const s = String(left % 60).padStart(2, '0')
  const text = h > 0 ? `${h}h${String(m).padStart(2, '0')}` : `${m}:${s}`
  return <Badge className="bg-warning font-mono text-black tabular-nums">Activée · {text}</Badge>
}

function EnableMenu({ route }: { route: RouteView }) {
  const queryClient = useQueryClient()
  const enable = useMutation({
    mutationFn: (minutes: number | null) => api.enableRoute(route, minutes),
    onSuccess: () => {
      toast.success(`Route « ${routeId(route)} » activée`)
      queryClient.invalidateQueries({ queryKey: ['routes'] })
    },
    onError: (e) => toast.error(e.message),
  })

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" disabled={enable.isPending}>
          <Fingerprint />
          {enable.isPending ? 'En attente de Touch ID…' : 'Activer'}
          {!enable.isPending && <ChevronDown className="opacity-60" />}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel className="text-muted-foreground text-xs">Activer pendant</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {DURATIONS.map((d) => (
          <DropdownMenuItem key={d.label} onSelect={() => enable.mutate(d.minutes)}>
            {d.label}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function DisableButton({ route }: { route: RouteView }) {
  const queryClient = useQueryClient()
  const disable = useMutation({
    mutationFn: () => api.disableRoute(route),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['routes'] }),
    onError: (e) => toast.error(e.message),
  })
  return (
    <Button size="sm" variant="outline" onClick={() => disable.mutate()} disabled={disable.isPending}>
      <Power /> Désactiver
    </Button>
  )
}
