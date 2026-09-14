/**
 * 解密页面展示工具类。
 *
 * 这些方法只服务于 DecryptPageView.vue 的模板表达式，
 * 目的是把类型转换和进度计算集中到一个可读的位置。
 */
export class DecryptPageViewUtils {
  /**
   * 将 Element Plus 输入框事件值转换为字符串。
   *
   * Element Plus 的事件类型可能来自字符串、数字或布尔值，
   * 这里统一转成路径输入框需要的字符串，避免模板里出现隐式转换。
   */
  static toTextInputValue(raw_value: string | number | boolean | undefined): string {
    if (raw_value === undefined) {
      return ''
    }

    return String(raw_value)
  }

  /**
   * 将 Element Plus 滑块事件值转换为数字。
   *
   * 滑块当前配置为单值模式，但组件类型仍可能给出数组或字符串，
   * 所以这里集中处理异常输入，保证父组件收到稳定的数字。
   */
  static toSliderNumberValue(raw_value: string | number | boolean | number[] | undefined): number {
    if (Array.isArray(raw_value)) {
      return Number(raw_value[0] ?? 1)
    }

    if (raw_value === undefined) {
      return 1
    }

    return Number(raw_value)
  }

  /**
   * 根据当前进度和总数计算百分比。
   *
   * 当总数为 0 或负数时直接返回 0，避免出现除零或 NaN。
   */
  static calculateProgressPercentage(current_count: number, total_count: number): number {
    if (total_count <= 0) {
      return 0
    }

    return Math.round((current_count / total_count) * 100)
  }

  /**
   * 根据任务状态返回 Element Plus 进度条状态。
   *
   * 任务运行中不显示完成状态，任务结束后显示 success。
   */
  static getProgressStatus(is_decrypting: boolean): '' | 'success' {
    if (is_decrypting) {
      return ''
    }

    return 'success'
  }

  /**
   * 将 Vue 模板 ref 的原始值收窄为 HTMLElement。
   *
   * Vue 的模板 ref 可能给出组件实例、真实 DOM 或 null，
   * 父组件只需要真实 DOM 节点来执行日志自动滚动。
   */
  static toHtmlElement(raw_element: Element | object | null): HTMLElement | null {
    if (raw_element instanceof HTMLElement) {
      return raw_element
    }

    return null
  }
}
