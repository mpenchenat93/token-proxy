import { createContext, useContext, useEffect, useState } from 'react'

export type Theme = 'light' | 'dark' | 'system'

const STORAGE_KEY = 'token-proxy-theme'
const DARK_QUERY = '(prefers-color-scheme: dark)'

/** Runs before the first paint to avoid a flash of the wrong theme. */
export const THEME_INIT_SCRIPT = `(function(){var t;try{t=localStorage.getItem('${STORAGE_KEY}')}catch(e){}var d=t==='dark'||(t!=='light'&&matchMedia('${DARK_QUERY}').matches);document.documentElement.classList.toggle('dark',d)})()`

function readTheme(): Theme {
  try {
    const value = localStorage.getItem(STORAGE_KEY)
    if (value === 'light' || value === 'dark' || value === 'system') return value
  } catch {
    // Storage unavailable: fall back to the system appearance.
  }
  return 'system'
}

function isDark(theme: Theme): boolean {
  return theme === 'dark' || (theme === 'system' && matchMedia(DARK_QUERY).matches)
}

interface ThemeContextValue {
  theme: Theme
  resolved: 'light' | 'dark'
  setTheme: (theme: Theme) => void
}

const ThemeContext = createContext<ThemeContextValue | null>(null)

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme, setThemeState] = useState<Theme>('system')
  const [resolved, setResolved] = useState<'light' | 'dark'>('light')

  useEffect(() => setThemeState(readTheme()), [])

  useEffect(() => {
    const apply = () => {
      const dark = isDark(theme)
      document.documentElement.classList.toggle('dark', dark)
      setResolved(dark ? 'dark' : 'light')
    }
    apply()
    if (theme !== 'system') return
    const media = matchMedia(DARK_QUERY)
    media.addEventListener('change', apply)
    return () => media.removeEventListener('change', apply)
  }, [theme])

  const setTheme = (next: Theme) => {
    try {
      localStorage.setItem(STORAGE_KEY, next)
    } catch {
      // Not persisted; still applied for this page.
    }
    setThemeState(next)
  }

  return <ThemeContext.Provider value={{ theme, resolved, setTheme }}>{children}</ThemeContext.Provider>
}

export function useTheme(): ThemeContextValue {
  const value = useContext(ThemeContext)
  if (!value) throw new Error('useTheme must be used inside ThemeProvider')
  return value
}
