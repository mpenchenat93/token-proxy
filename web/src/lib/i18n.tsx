import { createContext, useContext, useEffect, useState } from 'react'

// UI translations. Messages coming from the server (API errors, Touch ID
// reasons) are not translated here: the server writes them in the macOS
// language.

export type Locale = 'fr' | 'en'

export const LOCALES: { value: Locale; label: string }[] = [
  { value: 'en', label: 'English' },
  { value: 'fr', label: 'Français' },
]

const fr = {
  adminUnreachable: 'API d’administration injoignable',
  proxyOn: 'Proxy sur',
  copyProxyUrl: 'Copier l’URL du proxy',
  proxyUrlCopied: 'URL du proxy copiée',
  proxyStopped: (error: string) => `Proxy arrêté : ${error}`,
  starting: 'Démarrage…',
  addRoute: 'Ajouter une route',
  theme: 'Thème',
  themeLight: 'Clair',
  themeDark: 'Sombre',
  themeSystem: 'Système',
  language: 'Langue',
  approvalPending:
    'Une demande Touch ID est en attente. N’approuve que ce que tu as demandé toi-même.',
  routesTitle: 'Routes',
  routesDescription:
    'L’IA appelle /<groupe>/<route>/… : le groupe dit à qui appartient le compte. Chaque route démarre désactivée. Activer, ajouter, modifier ou supprimer une route demande Touch ID ; la désactiver, jamais.',
  allGroups: 'Tous les groupes',
  loading: 'Chargement…',
  noRoutes: 'Aucune route pour l’instant. Enregistre un premier token pour commencer.',
  recentRequests: 'Requêtes récentes',
  recentRequestsDescription:
    'Depuis le démarrage du service. Les 30 derniers jours sont gardés dans ~/Library/Logs/token-proxy/audit.log',
  noRequests: 'Aucune requête pour l’instant.',
  vaultReloaded: 'Coffre chargé, les routes sont de retour',
  waitingForKeychain: 'En attente du Trousseau…',
  retry: 'Réessayer',
  vaultError: (error: string) =>
    `Le coffre du Trousseau n’a pas pu être lu : aucune route n’est disponible (${error}). Clique sur « Réessayer », puis sur « Toujours autoriser » dans la demande de macOS.`,
  columnRoute: 'Route',
  columnService: 'Service',
  columnState: 'État',
  columnActions: 'Actions',
  routeCount: (n: number) => `${n} route${n > 1 ? 's' : ''}`,
  moreActions: 'Plus d’actions',
  edit: 'Modifier',
  delete: 'Supprimer',
  cancel: 'Annuler',
  save: 'Enregistrer',
  waitingForTouchId: 'En attente de Touch ID…',
  deleteTitle: (id: string) => `Supprimer « ${id} » ?`,
  deleteDescription: 'La route et son token sont retirés du Trousseau. Cette action est irréversible.',
  routeDeleted: (id: string) => `Route « ${id} » supprimée`,
  disabled: 'Désactivée',
  enabled: 'Activée',
  enable: 'Activer',
  enableFor: 'Activer pendant',
  disable: 'Désactiver',
  routeEnabled: (id: string) => `Route « ${id} » activée`,
  duration15m: '15 minutes',
  duration1h: '1 heure',
  duration4h: '4 heures',
  untilDisabled: 'Jusqu’à désactivation',
  routeSaved: (id: string) => `Route « ${id} » enregistrée`,
  routeSavedDescription: 'Elle reste désactivée tant que tu ne l’actives pas.',
  editTitle: (id: string) => `Modifier « ${id} »`,
  newRoute: 'Nouvelle route',
  formDescription:
    'Le token est stocké dans le Trousseau macOS et n’est plus jamais affiché. Ce que la route peut faire dépend des permissions du token. L’enregistrement demande Touch ID.',
  group: 'Groupe',
  groupHint: 'perso, ou un client',
  name: 'Nom',
  nameHint: 'Le service, sans le groupe',
  aiWillCall: 'L’IA appellera',
  namePlaceholder: '<nom>',
  serviceUrl: 'URL du service',
  token: 'Token',
  keepToken: 'Laisser vide pour garder le token actuel',
  pasteTokenAgain: 'L’URL ou la transmission a changé : colle le token à nouveau',
  pasteToken: 'Colle le token',
  tokenTransmission: 'Transmission du token',
  customHeader: 'En-tête personnalisé',
  queryParameter: 'Paramètre d’URL',
  headerName: 'Nom de l’en-tête',
  parameterName: 'Nom du paramètre',
  description: 'Description',
  descriptionHint: 'Affichée à l’IA quand elle liste les routes',
}

type Messages = typeof fr

const en: Messages = {
  adminUnreachable: 'Admin API unreachable',
  proxyOn: 'Proxy on',
  copyProxyUrl: 'Copy the proxy URL',
  proxyUrlCopied: 'Proxy URL copied',
  proxyStopped: (error) => `Proxy stopped: ${error}`,
  starting: 'Starting…',
  addRoute: 'Add a route',
  theme: 'Theme',
  themeLight: 'Light',
  themeDark: 'Dark',
  themeSystem: 'System',
  language: 'Language',
  approvalPending: 'A Touch ID request is pending. Only approve what you asked for yourself.',
  routesTitle: 'Routes',
  routesDescription:
    'The AI calls /<group>/<route>/…: the group says whose account it is. Every route starts disabled. Enabling, adding, editing or deleting a route needs Touch ID; disabling one never does.',
  allGroups: 'All groups',
  loading: 'Loading…',
  noRoutes: 'No routes yet. Save a first token to get started.',
  recentRequests: 'Recent requests',
  recentRequestsDescription:
    'Since the service started. The last 30 days are kept in ~/Library/Logs/token-proxy/audit.log',
  noRequests: 'No requests yet.',
  vaultReloaded: 'Vault loaded, the routes are back',
  waitingForKeychain: 'Waiting for the Keychain…',
  retry: 'Retry',
  vaultError: (error) =>
    `The Keychain vault could not be read: no route is available (${error}). Click "Retry", then "Always Allow" in the macOS prompt.`,
  columnRoute: 'Route',
  columnService: 'Service',
  columnState: 'State',
  columnActions: 'Actions',
  routeCount: (n) => `${n} route${n > 1 ? 's' : ''}`,
  moreActions: 'More actions',
  edit: 'Edit',
  delete: 'Delete',
  cancel: 'Cancel',
  save: 'Save',
  waitingForTouchId: 'Waiting for Touch ID…',
  deleteTitle: (id) => `Delete "${id}"?`,
  deleteDescription: 'The route and its token are removed from the Keychain. This cannot be undone.',
  routeDeleted: (id) => `Route "${id}" deleted`,
  disabled: 'Disabled',
  enabled: 'Enabled',
  enable: 'Enable',
  enableFor: 'Enable for',
  disable: 'Disable',
  routeEnabled: (id) => `Route "${id}" enabled`,
  duration15m: '15 minutes',
  duration1h: '1 hour',
  duration4h: '4 hours',
  untilDisabled: 'Until disabled',
  routeSaved: (id) => `Route "${id}" saved`,
  routeSavedDescription: 'It stays disabled until you enable it.',
  editTitle: (id) => `Edit "${id}"`,
  newRoute: 'New route',
  formDescription:
    'The token is stored in the macOS Keychain and never shown again. What the route can do depends on the token’s permissions. Saving needs Touch ID.',
  group: 'Group',
  groupHint: 'perso, or a client',
  name: 'Name',
  nameHint: 'The service, without the group',
  aiWillCall: 'The AI will call',
  namePlaceholder: '<name>',
  serviceUrl: 'Service URL',
  token: 'Token',
  keepToken: 'Leave empty to keep the current token',
  pasteTokenAgain: 'The URL or how the token is sent changed: paste the token again',
  pasteToken: 'Paste the token',
  tokenTransmission: 'How the token is sent',
  customHeader: 'Custom header',
  queryParameter: 'Query parameter',
  headerName: 'Header name',
  parameterName: 'Parameter name',
  description: 'Description',
  descriptionHint: 'Shown to the AI when it lists routes',
}

const MESSAGES: Record<Locale, Messages> = { fr, en }

/** BCP 47 tag used to format dates and times. */
export const DATE_LOCALE: Record<Locale, string> = { fr: 'fr-FR', en: 'en-GB' }

const STORAGE_KEY = 'token-proxy-locale'

/** The saved choice, else French for a French browser, else English. */
function detectLocale(): Locale {
  if (typeof window === 'undefined') return 'en'
  try {
    const saved = localStorage.getItem(STORAGE_KEY)
    if (saved === 'fr' || saved === 'en') return saved
  } catch {
    // Storage unavailable: use the browser language.
  }
  const preferred = navigator.languages?.[0] ?? navigator.language ?? ''
  return preferred.toLowerCase().startsWith('fr') ? 'fr' : 'en'
}

interface I18nContextValue {
  locale: Locale
  t: Messages
  setLocale: (locale: Locale) => void
}

const I18nContext = createContext<I18nContextValue | null>(null)

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(detectLocale)

  useEffect(() => {
    document.documentElement.lang = locale
  }, [locale])

  const setLocale = (next: Locale) => {
    try {
      localStorage.setItem(STORAGE_KEY, next)
    } catch {
      // Not persisted; still applied for this page.
    }
    setLocaleState(next)
  }

  return (
    <I18nContext.Provider value={{ locale, t: MESSAGES[locale], setLocale }}>
      {children}
    </I18nContext.Provider>
  )
}

export function useI18n(): I18nContextValue {
  const value = useContext(I18nContext)
  if (!value) throw new Error('useI18n must be used inside I18nProvider')
  return value
}
