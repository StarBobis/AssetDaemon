import { ref } from 'vue'
import { load } from '@tauri-apps/plugin-store'

export type AppTheme = 'light' | 'dark'
export type AppColorScheme =
  | 'lime'
  | 'titian'
  | 'mars'
  | 'klein'
  | 'burgundy'
  | 'schonbrunn'
  | 'tiffany'
  | 'china-red'
  | 'vandyke'
  | 'hermes'
  | 'prussian'

export type ColorSchemeOption = {
  value: AppColorScheme
  labelKey: string
  descriptionKey: string
  colors: string[]
}

export const COLOR_SCHEME_OPTIONS: ColorSchemeOption[] = [
  {
    value: 'lime',
    labelKey: 'settings.colorSchemeLime',
    descriptionKey: 'settings.colorSchemeLimeDescription',
    colors: ['#6ECC54', '#018B8D', '#F9D46C', '#0D3A69'],
  },
  {
    value: 'titian',
    labelKey: 'settings.colorSchemeTitian',
    descriptionKey: 'settings.colorSchemeTitianDescription',
    colors: ['#D34947', '#C8161D', '#F9D46C', '#470125'],
  },
  {
    value: 'mars',
    labelKey: 'settings.colorSchemeMars',
    descriptionKey: 'settings.colorSchemeMarsDescription',
    colors: ['#018B8D', '#71E2D1', '#F9D46C', '#0D3A69'],
  },
  {
    value: 'klein',
    labelKey: 'settings.colorSchemeKlein',
    descriptionKey: 'settings.colorSchemeKleinDescription',
    colors: ['#002FA7', '#71E2D1', '#F9D46C', '#0D3A69'],
  },
  {
    value: 'burgundy',
    labelKey: 'settings.colorSchemeBurgundy',
    descriptionKey: 'settings.colorSchemeBurgundyDescription',
    colors: ['#470125', '#D34947', '#F9D46C', '#492D22'],
  },
  {
    value: 'schonbrunn',
    labelKey: 'settings.colorSchemeSchonbrunn',
    descriptionKey: 'settings.colorSchemeSchonbrunnDescription',
    colors: ['#F9D46C', '#EB5C20', '#018B8D', '#492D22'],
  },
  {
    value: 'tiffany',
    labelKey: 'settings.colorSchemeTiffany',
    descriptionKey: 'settings.colorSchemeTiffanyDescription',
    colors: ['#71E2D1', '#018B8D', '#6ECC54', '#0D3A69'],
  },
  {
    value: 'china-red',
    labelKey: 'settings.colorSchemeChinaRed',
    descriptionKey: 'settings.colorSchemeChinaRedDescription',
    colors: ['#C8161D', '#D34947', '#F9D46C', '#470125'],
  },
  {
    value: 'vandyke',
    labelKey: 'settings.colorSchemeVandyke',
    descriptionKey: 'settings.colorSchemeVandykeDescription',
    colors: ['#492D22', '#EB5C20', '#F9D46C', '#018B8D'],
  },
  {
    value: 'hermes',
    labelKey: 'settings.colorSchemeHermes',
    descriptionKey: 'settings.colorSchemeHermesDescription',
    colors: ['#EB5C20', '#F9D46C', '#492D22', '#018B8D'],
  },
  {
    value: 'prussian',
    labelKey: 'settings.colorSchemePrussian',
    descriptionKey: 'settings.colorSchemePrussianDescription',
    colors: ['#0D3A69', '#018B8D', '#71E2D1', '#F9D46C'],
  },
]

function normalizeColorScheme(value: unknown): AppColorScheme {
  if (value === 'prussian-mars') return 'prussian'
  if (value === 'tiffany-lime') return 'tiffany'
  if (value === 'hermes-schonbrunn') return 'hermes'
  if (value === 'burgundy-titian') return 'burgundy'
  return COLOR_SCHEME_OPTIONS.some((option) => option.value === value)
    ? value as AppColorScheme
    : 'lime'
}

export class ThemeController {
  theme = ref<AppTheme>('dark')
  colorScheme = ref<AppColorScheme>('lime')

  private store_promise: ReturnType<typeof load> | null = null

  constructor() {
    this.loadTheme = this.loadTheme.bind(this)
    this.setTheme = this.setTheme.bind(this)
    this.setColorScheme = this.setColorScheme.bind(this)
  }

  async loadTheme(): Promise<void> {
    try {
      const store = await this.getStore()
      const saved_theme = await store.get('app-theme')
      const saved_color_scheme = await store.get('app-color-scheme')
      this.applyTheme(saved_theme === 'light' || saved_theme === 'dark' ? saved_theme : 'dark')
      this.applyColorScheme(normalizeColorScheme(saved_color_scheme))
    } catch {
      this.applyTheme('dark')
      this.applyColorScheme('lime')
    }
  }

  async setTheme(value: AppTheme): Promise<void> {
    this.applyTheme(value)
    const store = await this.getStore()
    await store.set('app-theme', value)
    await store.save()
  }

  async setColorScheme(value: AppColorScheme): Promise<void> {
    const next_scheme = normalizeColorScheme(value)
    this.applyColorScheme(next_scheme)
    const store = await this.getStore()
    await store.set('app-color-scheme', next_scheme)
    await store.save()
  }

  private getStore(): ReturnType<typeof load> {
    if (!this.store_promise) {
      this.store_promise = load('settings.json', { defaults: {}, autoSave: true })
    }

    return this.store_promise
  }

  private applyTheme(value: AppTheme): void {
    this.theme.value = value
    const root = document.documentElement
    root.dataset.theme = value
    root.classList.toggle('dark', value === 'dark')
    root.style.colorScheme = value
  }

  private applyColorScheme(value: AppColorScheme): void {
    this.colorScheme.value = value
    document.documentElement.dataset.colorScheme = value
  }
}

const theme_controller = new ThemeController()

export function useTheme() {
  return {
    theme: theme_controller.theme,
    colorScheme: theme_controller.colorScheme,
    colorSchemeOptions: COLOR_SCHEME_OPTIONS,
    loadTheme: theme_controller.loadTheme,
    setTheme: theme_controller.setTheme,
    setColorScheme: theme_controller.setColorScheme,
  }
}
