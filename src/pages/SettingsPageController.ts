import { computed, type Ref, ref } from 'vue'
import { load } from '@tauri-apps/plugin-store'
import { open } from '@tauri-apps/plugin-dialog'
import { I18nLanguageUtils, type SupportedLocale } from '../i18n'
import type { AppColorScheme, AppTheme } from '../composables/useTheme'
import { AssetMapCacheUtils, ASSET_MAP_CACHE_ROOT_KEY } from '../utils/AssetMapCacheUtils'
import { GITHUB_TOKEN_STORE_KEY } from '../utils/UpdateChecker'

/**
 * 设置页面控制器初始化参数。
 *
 * 这些参数来自 Vue composable 和 vue-i18n，控制器只持有需要的最小接口。
 */
interface SettingsPageControllerOptions {
  locale: Ref<string>
  loadTheme: () => Promise<void>
  setTheme: (value: AppTheme) => Promise<void>
  setColorScheme: (value: AppColorScheme) => Promise<void>
}

/**
 * 设置页面控制器。
 *
 * 该类集中处理设置页的持久化读取、写入、语言切换和滑块事件。
 * Settings.vue 只负责把状态绑定给模板，不再直接定义独立函数。
 */
export class SettingsPageController {
  /** 类型扫描并发数量，默认和历史设置保持一致。 */
  concurrency = ref(32)

  /** Bundle 元数据缓存上限，单位为 GB，供滑块展示。 */
  cache_max_gb = ref(8)

  /** Bundle 元数据缓存上限，单位为字节，供业务模块读取。 */
  cache_max_bytes = ref(8 * 1024 * 1024 * 1024)

  asset_map_cache_root = ref('')

  github_token = ref('')

  /** 当前选择的界面语言。 */
  selected_language = computed(() => this.options.locale.value as SupportedLocale)

  /** 持久化 store 实例，使用懒加载避免顶层 await。 */
  private store: Awaited<ReturnType<typeof load>> | null = null

  constructor(private readonly options: SettingsPageControllerOptions) {
    this.loadSettings = this.loadSettings.bind(this)
    this.handleThemeChange = this.handleThemeChange.bind(this)
    this.handleColorSchemeChange = this.handleColorSchemeChange.bind(this)
    this.setConcurrency = this.setConcurrency.bind(this)
    this.setCacheMaxGB = this.setCacheMaxGB.bind(this)
    this.selectAssetMapCacheRoot = this.selectAssetMapCacheRoot.bind(this)
    this.copyAssetMapCacheRoot = this.copyAssetMapCacheRoot.bind(this)
    this.setLanguage = this.setLanguage.bind(this)
    this.setGithubToken = this.setGithubToken.bind(this)
  }

  /**
   * 页面挂载后加载全部设置。
   *
   * 异步读取放在 onMounted 中触发，避免 <script setup> 顶层 await。
   */
  async loadSettings(): Promise<void> {
    await this.options.loadTheme()
    await this.loadConcurrency()
    await this.loadAssetMapCacheRoot()
    await this.loadCacheConfig()
    await this.loadGithubToken()
  }

  /**
   * 处理主题切换。
   *
   * Element Plus 可能传入字符串或数字，这里统一转成项目支持的主题值。
   */
  handleThemeChange(value: string | number): void {
    const next_theme = value === 'light' ? 'light' : 'dark'
    void this.options.setTheme(next_theme)
  }

  handleColorSchemeChange(value: string | number): void {
    const next_scheme = String(value) as AppColorScheme
    void this.options.setColorScheme(next_scheme)
  }

  /**
   * 保存类型扫描并发数量。
   *
   * 该值会影响后续扫描任务的并行度，所以用户调整后立即持久化。
   */
  async setConcurrency(value: number): Promise<void> {
    this.concurrency.value = value
    const store = await this.getStore()
    await store.set('scan-concurrency', value)
  }

  /**
   * 保存 Bundle 元数据缓存上限。
   *
   * UI 使用 GB，持久化和业务逻辑使用 bytes，因此这里负责单位转换。
   */
  async setCacheMaxGB(value: number): Promise<void> {
    this.cache_max_gb.value = value
    const bytes = value * 1024 * 1024 * 1024
    this.cache_max_bytes.value = bytes

    const store = await this.getStore()
    await store.set('bundle-cache-max-bytes', bytes)
    await store.save()
  }

  async selectAssetMapCacheRoot(): Promise<void> {
    const selected = await open({
      directory: true,
      multiple: false,
      title: this.asset_map_cache_root.value || undefined,
    })
    if (!selected || typeof selected !== 'string') return

    this.asset_map_cache_root.value = selected
    const store = await this.getStore()
    await store.set(ASSET_MAP_CACHE_ROOT_KEY, selected)
    await store.save()
  }

  async copyAssetMapCacheRoot(): Promise<void> {
    if (!this.asset_map_cache_root.value) return
    await navigator.clipboard.writeText(this.asset_map_cache_root.value)
  }

  /**
   * 保存界面语言。
   *
   * 当前只支持中文和英文，非法输入会回退为英文。
   */
  async setLanguage(value: string | number): Promise<void> {
    const next_locale = value === 'zh' ? 'zh' : 'en'
    this.options.locale.value = next_locale
    await I18nLanguageUtils.persistLocale(next_locale)
  }

  async setGithubToken(value: string): Promise<void> {
    this.github_token.value = value
    const store = await this.getStore()
    await store.set(GITHUB_TOKEN_STORE_KEY, value)
    await store.save()
  }

  /**
   * 获取持久化 store。
   *
   * 首次调用时创建，后续复用，减少重复 IPC。
   */
  private async getStore(): Promise<Awaited<ReturnType<typeof load>>> {
    if (!this.store) {
      this.store = await load('settings.json', { defaults: {}, autoSave: true })
    }

    return this.store
  }

  /**
   * 从持久化设置中恢复扫描并发数量。
   *
   * 只接受 1 到 32 的合法范围，避免旧数据或手工编辑造成异常。
   */
  private async loadConcurrency(): Promise<void> {
    const store = await this.getStore()
    const saved_value = await store.get('scan-concurrency')

    if (typeof saved_value === 'number' && saved_value >= 1 && saved_value <= 32) {
      this.concurrency.value = saved_value
    }
  }

  /**
   * 从持久化设置中恢复缓存上限。
   *
   * 存储层保存 bytes，界面层展示 GB，因此读取后做一次换算和范围限制。
   */
  private async loadCacheConfig(): Promise<void> {
    const store = await this.getStore()
    const saved_value = await store.get('bundle-cache-max-bytes')

    if (typeof saved_value === 'number' && saved_value > 0) {
      const gigabytes = Math.round(saved_value / (1024 * 1024 * 1024))
      this.cache_max_gb.value = Math.max(1, Math.min(gigabytes, 48))
      this.cache_max_bytes.value = this.cache_max_gb.value * 1024 * 1024 * 1024
    }
  }

  private async loadAssetMapCacheRoot(): Promise<void> {
    this.asset_map_cache_root.value = await AssetMapCacheUtils.getCacheRoot()
  }

  private async loadGithubToken(): Promise<void> {
    const store = await this.getStore()
    const token = await store.get(GITHUB_TOKEN_STORE_KEY)
    this.github_token.value = typeof token === 'string' ? token : ''
  }
}
