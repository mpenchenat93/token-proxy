import { Check, Monitor, Moon, Sun } from 'lucide-react'
import { Button } from '#/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '#/components/ui/dropdown-menu'
import { useI18n } from '#/lib/i18n'
import { useTheme } from '#/lib/theme'
import type { Theme } from '#/lib/theme'

const OPTIONS: { value: Theme; label: 'themeLight' | 'themeDark' | 'themeSystem'; icon: typeof Sun }[] = [
  { value: 'light', label: 'themeLight', icon: Sun },
  { value: 'dark', label: 'themeDark', icon: Moon },
  { value: 'system', label: 'themeSystem', icon: Monitor },
]

export function ThemeToggle() {
  const { theme, resolved, setTheme } = useTheme()
  const { t } = useI18n()
  const Icon = resolved === 'dark' ? Moon : Sun

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="icon" aria-label={t.theme}>
          <Icon />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {OPTIONS.map(({ value, label, icon: ItemIcon }) => (
          <DropdownMenuItem key={value} onSelect={() => setTheme(value)}>
            <ItemIcon /> {t[label]}
            {theme === value && <Check className="ml-auto" />}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
