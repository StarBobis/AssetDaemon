import { createI18n } from 'vue-i18n'
import type { Ref } from 'vue'
import { load } from '@tauri-apps/plugin-store'
import en from './locales/en.json'
import zh from './locales/zh.json'

export type SupportedLocale = 'en' | 'zh'

const languageStoreKey = 'app-language'
const legacyLanguageStorageKey = 'assetfinder-language'

export class I18nLanguageUtils {
  private static store_promise: ReturnType<typeof load> | null = null

  /**
   * Normalize any external locale value into the app-supported locale list.
   *
   * Unknown values fall back to English so the app always has a valid message bundle.
   */
  static normalizeLocale(locale?: string | null): SupportedLocale {
    if (locale === 'en' || locale === 'zh') {
      return locale
    }

    return 'en'
  }

  static getDefaultLocale(): SupportedLocale {
    return I18nLanguageUtils.normalizeLocale(I18nLanguageUtils.getLegacyLocale())
  }

  static async loadPersistedLocale(locale: Ref<string>): Promise<void> {
    const saved_locale = await I18nLanguageUtils.getStoredLocale()
    if (saved_locale) {
      locale.value = saved_locale
      return
    }

    const legacy_locale = I18nLanguageUtils.getLegacyLocale()
    if (legacy_locale === 'en' || legacy_locale === 'zh') {
      locale.value = legacy_locale
      await I18nLanguageUtils.persistLocale(legacy_locale)
    }
  }

  static async persistLocale(locale: SupportedLocale): Promise<void> {
    const next_locale = I18nLanguageUtils.normalizeLocale(locale)

    try {
      const store = await I18nLanguageUtils.getStore()
      await store.set(languageStoreKey, next_locale)
      await store.save()
      I18nLanguageUtils.removeLegacyLocale()
    } catch (error) {
      console.error('Failed to persist app language:', error)
      I18nLanguageUtils.setLegacyLocale(next_locale)
    }
  }

  private static getLegacyStorage(): Storage | null {
    return typeof localStorage === 'undefined' ? null : localStorage
  }

  private static getLegacyLocale(): string | null {
    return I18nLanguageUtils.getLegacyStorage()?.getItem(legacyLanguageStorageKey) ?? null
  }

  private static setLegacyLocale(locale: SupportedLocale): void {
    I18nLanguageUtils.getLegacyStorage()?.setItem(legacyLanguageStorageKey, locale)
  }

  private static removeLegacyLocale(): void {
    I18nLanguageUtils.getLegacyStorage()?.removeItem(legacyLanguageStorageKey)
  }

  private static getStore(): ReturnType<typeof load> {
    if (!I18nLanguageUtils.store_promise) {
      I18nLanguageUtils.store_promise = load('settings.json', { defaults: {}, autoSave: true })
    }

    return I18nLanguageUtils.store_promise
  }

  private static async getStoredLocale(): Promise<SupportedLocale | null> {
    try {
      const store = await I18nLanguageUtils.getStore()
      const value = await store.get(languageStoreKey)
      return value === 'en' || value === 'zh' ? value : null
    } catch (error) {
      console.error('Failed to load app language:', error)
      return null
    }
  }
}

export const i18n = createI18n({
  legacy: false,
  locale: I18nLanguageUtils.getDefaultLocale(),
  fallbackLocale: 'en',
  messages: {
    en,
    zh,
  },
})
