import { ref, type Ref, watch } from 'vue'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { openUrl } from '@tauri-apps/plugin-opener'
import { WindowUtils } from '../utils/WindowUtils'

/**
 * 自定义标题栏控制器参数。
 *
 * 路由和当前路由由组件传入，控制器只负责动作编排。
 */
interface CustomTitlebarControllerOptions {
  router: Router
  route: RouteLocationNormalizedLoaded
}

/**
 * 自定义标题栏控制器。
 *
 * 该类集中处理页面跳转、临时页面返回逻辑、窗口最小化/最大化/关闭和问题反馈链接。
 */
export class CustomTitlebarController {
  /** 当前窗口是否最大化。 */
  is_maximized = ref(false)

  /** 从临时页面返回时使用的原路径。 */
  return_path = ref('/')

  /** GitHub 问题反馈地址。 */
  readonly issue_url = 'https://github.com/StarBobis/AssetDaemon/issues'

  /** Tauri 当前窗口句柄。 */
  private readonly app_window = getCurrentWindow()

  /** 可通过再次点击按钮返回的临时页面集合。 */
  private readonly temporary_pages = new Set(['/decrypt', '/encrypt', '/settings'])

  constructor(private readonly options: CustomTitlebarControllerOptions) {
    this.navigate = this.navigate.bind(this)
    this.toggleTemporaryPage = this.toggleTemporaryPage.bind(this)
    this.minimize = this.minimize.bind(this)
    this.toggleMaximize = this.toggleMaximize.bind(this)
    this.closeWindow = this.closeWindow.bind(this)
    this.openIssuePage = this.openIssuePage.bind(this)
    this.refreshMaximizedState = this.refreshMaximizedState.bind(this)

    this.watchRouteChanges(this.return_path)
    void this.app_window.listen('tauri://resize', this.refreshMaximizedState)
  }

  /**
   * 跳转到指定路由。
   *
   * 品牌按钮使用这个方法直接回到工作区首页。
   */
  navigate(path: string): void {
    void this.options.router.push(path)
  }

  /**
   * 打开或关闭临时页面。
   *
   * 如果当前已经在目标页面，则返回进入临时页面前的路径。
   */
  toggleTemporaryPage(path: string): void {
    if (this.options.route.path === path) {
      void this.options.router.push(this.return_path.value || '/')
      return
    }

    this.return_path.value = this.options.route.path || '/'
    void this.options.router.push(path)
  }

  /**
   * 最小化当前 Tauri 窗口。
   *
   * 失败时只记录错误，不阻断标题栏其它交互。
   */
  async minimize(): Promise<void> {
    try {
      await this.app_window.minimize()
    } catch (error) {
      console.error(error)
    }
  }

  /**
   * 在最大化和还原之间切换。
   *
   * 操作完成后同步更新本地状态，保证按钮图标立即变化。
   */
  async toggleMaximize(): Promise<void> {
    try {
      const is_currently_maximized = await this.app_window.isMaximized()

      if (is_currently_maximized) {
        await this.app_window.unmaximize()
        this.is_maximized.value = false
        return
      }

      await this.app_window.maximize()
      this.is_maximized.value = true
    } catch (error) {
      console.error(error)
    }
  }

  /**
   * 关闭当前 Tauri 窗口。
   *
   * 这里保持和原逻辑一致，不额外弹确认框。
   */
  async closeWindow(): Promise<void> {
    try {
      await WindowUtils.saveWindowBounds({ immediate: true })
      await this.app_window.close()
    } catch (error) {
      console.error(error)
    }
  }

  /**
   * 打开问题反馈页面。
   *
   * 使用 Tauri opener 插件交给系统默认浏览器处理。
   */
  async openIssuePage(): Promise<void> {
    try {
      await openUrl(this.issue_url)
    } catch (error) {
      console.error(error)
    }
  }

  /**
   * 刷新最大化状态。
   *
   * 窗口 resize 事件触发时调用，避免窗口状态和按钮图标不同步。
   */
  private async refreshMaximizedState(): Promise<void> {
    try {
      this.is_maximized.value = await this.app_window.isMaximized()
    } catch {
      // 读取状态失败时保持旧状态，避免标题栏抖动。
    }
  }

  /**
   * 监听路由变化以维护返回路径。
   *
   * 当用户从普通页面进入临时页面时，记录旧路径用于再次点击按钮返回。
   */
  private watchRouteChanges(return_path: Ref<string>): void {
    watch(
      () => this.options.route.path,
      (new_path, old_path) => {
        if (old_path && this.temporary_pages.has(new_path) && old_path !== new_path) {
          return_path.value = old_path
        }
      },
    )
  }
}
