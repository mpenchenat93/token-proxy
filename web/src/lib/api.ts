// Client for the token-proxy admin API (served by the Rust binary under /api).

export type AuthInjection =
  | 'Bearer'
  | { Header: { name: string } }
  | { Query: { param: string } }

export interface RouteView {
  /** Who the route belongs to: `perso`, a client… */
  group: string
  name: string
  upstream: string
  auth: AuthInjection
  description: string
  enabled: boolean
  expires_in_seconds: number | null
}

export interface RouteInput {
  group: string
  name: string
  upstream: string
  /** Required on creation; empty on update keeps the current token. */
  token?: string
  auth: AuthInjection
  description: string
}

export type ProxyStatus =
  | { state: 'starting' }
  | { state: 'listening'; url: string }
  | { state: 'failed'; error: string }

export interface Status {
  proxy: ProxyStatus
  vault_error: string | null
  approval_pending: boolean
}

export interface AuditEntry {
  at: string
  route: string
  method: string
  path: string
  status: number
  forwarded: boolean
  note: string
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(`/api${path}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  if (!response.ok) {
    const payload = await response.json().catch(() => null)
    throw new Error(payload?.error ?? `${response.status} ${response.statusText}`)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}

export const api = {
  status: () => request<Status>('GET', '/status'),
  routes: () => request<RouteView[]>('GET', '/routes'),
  audit: () => request<{ entries: AuditEntry[] }>('GET', '/audit'),
  createRoute: (input: RouteInput) => request<RouteView>('POST', '/routes', input),
  updateRoute: (route: RouteView, input: RouteInput) =>
    request<RouteView>('PUT', routePath(route), input),
  deleteRoute: (route: RouteView) => request<void>('DELETE', routePath(route)),
  enableRoute: (route: RouteView, minutes: number | null) =>
    request<RouteView>('POST', `${routePath(route)}/enable`, { minutes }),
  disableRoute: (route: RouteView) => request<RouteView>('POST', `${routePath(route)}/disable`),
  reloadVault: () => request<{ reloaded: boolean }>('POST', '/vault/reload'),
}

/** Group of new routes, listed first. */
export const DEFAULT_GROUP = 'perso'

/** `<group>/<name>`: what the AI puts in the URL. */
export function routeId(route: Pick<RouteView, 'group' | 'name'>): string {
  return `${route.group}/${route.name}`
}

function routePath(route: RouteView): string {
  return `/routes/${encodeURIComponent(route.group)}/${encodeURIComponent(route.name)}`
}

/** Group names, the personal one first. */
export function groupsOf(routes: RouteView[]): string[] {
  return [...new Set(routes.map((r) => r.group))].sort((a, b) =>
    a === DEFAULT_GROUP ? -1 : b === DEFAULT_GROUP ? 1 : a.localeCompare(b),
  )
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return url
  }
}
