import { useEffect, useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { Fingerprint, Loader2 } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '#/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '#/components/ui/dialog'
import { Input } from '#/components/ui/input'
import { Label } from '#/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '#/components/ui/select'
import { Textarea } from '#/components/ui/textarea'
import { api, DEFAULT_GROUP, routeId } from '#/lib/api'
import { useI18n } from '#/lib/i18n'
import type { AuthInjection, RouteInput, RouteView } from '#/lib/api'

type AuthMode = 'bearer' | 'header' | 'query'

interface FormState {
  group: string
  name: string
  upstream: string
  token: string
  authMode: AuthMode
  authValue: string
  description: string
}

function splitAuth(auth: AuthInjection): [AuthMode, string] {
  if (auth === 'Bearer') return ['bearer', '']
  if ('Header' in auth) return ['header', auth.Header.name]
  return ['query', auth.Query.param]
}

const EMPTY_FORM: FormState = {
  group: DEFAULT_GROUP,
  name: '',
  upstream: '',
  token: '',
  authMode: 'bearer',
  authValue: '',
  description: '',
}

function fromRoute(route: RouteView): FormState {
  const [authMode, authValue] = splitAuth(route.auth)
  return {
    group: route.group,
    name: route.name,
    upstream: route.upstream,
    token: '',
    authMode,
    authValue,
    description: route.description,
  }
}

function toInput(form: FormState): RouteInput {
  const auth: AuthInjection =
    form.authMode === 'bearer'
      ? 'Bearer'
      : form.authMode === 'header'
        ? { Header: { name: form.authValue.trim().toLowerCase() } }
        : { Query: { param: form.authValue.trim() } }
  return {
    group: form.group.trim(),
    name: form.name.trim(),
    upstream: form.upstream.trim(),
    token: form.token.trim() || undefined,
    auth,
    description: form.description.trim(),
  }
}

/** `scheme://host:port`, or the text itself while it is not a URL. */
function originOf(url: string): string {
  try {
    return new URL(url.trim()).origin
  } catch {
    return url.trim()
  }
}

/** Whether the token would go elsewhere than `route` sends it: the server then requires it again. */
function needsToken(form: FormState, route: RouteView | null): boolean {
  if (!route) return true
  const auth = JSON.stringify(toInput(form).auth)
  return originOf(form.upstream) !== originOf(route.upstream) || auth !== JSON.stringify(route.auth)
}

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Route to edit; `null` to create a new one. */
  route: RouteView | null
  /** Existing groups, offered as suggestions. */
  groups: string[]
}

export function RouteFormDialog({ open, onOpenChange, route, groups }: Props) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [form, setForm] = useState<FormState>(EMPTY_FORM)

  useEffect(() => {
    if (open) setForm(route ? fromRoute(route) : EMPTY_FORM)
  }, [open, route])

  const save = useMutation({
    mutationFn: (input: RouteInput) =>
      route ? api.updateRoute(route, input) : api.createRoute(input),
    onSuccess: (saved) => {
      queryClient.invalidateQueries({ queryKey: ['routes'] })
      toast.success(t.routeSaved(routeId(saved)), { description: t.routeSavedDescription })
      onOpenChange(false)
    },
  })

  useEffect(() => {
    if (!open) save.reset()
  }, [open])

  const tokenRequired = needsToken(form, route)

  const set = <K extends keyof FormState>(key: K, value: FormState[K]) =>
    setForm((f) => ({ ...f, [key]: value }))

  return (
    <Dialog open={open} onOpenChange={(o) => !save.isPending && onOpenChange(o)}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{route ? t.editTitle(routeId(route)) : t.newRoute}</DialogTitle>
          <DialogDescription>{t.formDescription}</DialogDescription>
        </DialogHeader>

        <form
          id="route-form"
          className="grid gap-4"
          onSubmit={(e) => {
            e.preventDefault()
            save.mutate(toInput(form))
          }}
        >
          <div className="grid gap-4 sm:grid-cols-2">
            <Field label={t.group} hint={t.groupHint}>
              <Input
                value={form.group}
                onChange={(e) => set('group', e.target.value)}
                placeholder={DEFAULT_GROUP}
                className="font-mono"
                list="route-groups"
                required
              />
              <datalist id="route-groups">
                {groups.map((g) => (
                  <option key={g} value={g} />
                ))}
              </datalist>
            </Field>
            <Field label={t.name} hint={t.nameHint}>
              <Input
                value={form.name}
                onChange={(e) => set('name', e.target.value)}
                placeholder="coolify"
                className="font-mono"
                autoFocus
                required
              />
            </Field>
          </div>
          <p className="text-muted-foreground -mt-2 text-xs">
            {t.aiWillCall}{' '}
            <span className="font-mono">
              /{form.group.trim() || DEFAULT_GROUP}/{form.name.trim() || t.namePlaceholder}/…
            </span>
          </p>

          <Field label={t.serviceUrl}>
            <Input
              value={form.upstream}
              onChange={(e) => set('upstream', e.target.value)}
              placeholder="https://coolify.example.com"
              required
            />
          </Field>

          <Field label={t.token}>
            <Input
              type="password"
              value={form.token}
              onChange={(e) => set('token', e.target.value)}
              placeholder={!route ? t.pasteToken : tokenRequired ? t.pasteTokenAgain : t.keepToken}
              autoComplete="off"
              required={tokenRequired}
            />
          </Field>

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label={t.tokenTransmission}>
              <Select value={form.authMode} onValueChange={(v) => set('authMode', v as AuthMode)}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="bearer">Authorization: Bearer</SelectItem>
                  <SelectItem value="header">{t.customHeader}</SelectItem>
                  <SelectItem value="query">{t.queryParameter}</SelectItem>
                </SelectContent>
              </Select>
            </Field>
            {form.authMode !== 'bearer' && (
              <Field label={form.authMode === 'header' ? t.headerName : t.parameterName}>
                <Input
                  value={form.authValue}
                  onChange={(e) => set('authValue', e.target.value)}
                  placeholder={form.authMode === 'header' ? 'x-api-key' : 'access_token'}
                  className="font-mono"
                  required
                />
              </Field>
            )}
          </div>

          <Field label={t.description} hint={t.descriptionHint}>
            <Textarea
              value={form.description}
              onChange={(e) => set('description', e.target.value)}
              className="min-h-16"
            />
          </Field>

          {save.error && <p className="text-destructive-foreground text-sm">{save.error.message}</p>}
        </form>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={save.isPending}>
            {t.cancel}
          </Button>
          <Button type="submit" form="route-form" disabled={save.isPending}>
            {save.isPending ? <Loader2 className="animate-spin" /> : <Fingerprint />}
            {save.isPending ? t.waitingForTouchId : t.save}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-1.5">
      <Label className="flex items-baseline gap-2">
        {label}
        {hint && <span className="text-muted-foreground text-xs font-normal">{hint}</span>}
      </Label>
      {children}
    </div>
  )
}
