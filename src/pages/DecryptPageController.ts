import { ref } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { load } from '@tauri-apps/plugin-store'
import { invoke } from '@tauri-apps/api/core'
import { DecryptGameName } from '../types'

type DecryptTranslateFunction = (key: string, named?: Record<string, unknown>) => string
type DecryptLogFunction = (message: string, type?: 'info' | 'success' | 'error' | 'warn') => void

export class DecryptPageController {
  selected_game = ref<DecryptGameName>(DecryptGameName.GirlsFrontline2)

  game_options = [
    DecryptGameName.GirlsFrontline2,
    DecryptGameName.NarakaBladepoint,
    DecryptGameName.TheMagicBlade,
  ]

  input_path = ref('')
  output_path = ref('')
  concurrency = ref(20)
  is_decrypting = ref(false)

  private store: Awaited<ReturnType<typeof load>> | null = null

  constructor(
    private readonly translate: DecryptTranslateFunction,
    private readonly add_global_log: DecryptLogFunction,
  ) {
    this.loadSavedSettings = this.loadSavedSettings.bind(this)
    this.selectInputFolder = this.selectInputFolder.bind(this)
    this.selectOutputFolder = this.selectOutputFolder.bind(this)
    this.handleInputPathChanged = this.handleInputPathChanged.bind(this)
    this.handleOutputPathChanged = this.handleOutputPathChanged.bind(this)
    this.handleSaveSelectedGame = this.handleSaveSelectedGame.bind(this)
    this.handleSaveConcurrency = this.handleSaveConcurrency.bind(this)
    this.startDecryption = this.startDecryption.bind(this)
    this.handleGlobalStopSignal = this.handleGlobalStopSignal.bind(this)
  }

  async loadSavedSettings(): Promise<void> {
    const store = await this.getStore()

    const saved_input = await store.get('decrypt-input-path')
    if (typeof saved_input === 'string') {
      this.input_path.value = saved_input
    }

    const saved_output = await store.get('decrypt-output-path')
    if (typeof saved_output === 'string') {
      this.output_path.value = saved_output
    }

    const saved_concurrency = await store.get('decrypt-concurrency')
    if (typeof saved_concurrency === 'number') {
      this.concurrency.value = Math.min(50, Math.max(1, Math.round(saved_concurrency)))
    }

    const saved_game = await store.get('decrypt-game-name')
    if (this.isDecryptGameName(saved_game)) {
      this.selected_game.value = saved_game
    }
  }

  async selectInputFolder(): Promise<void> {
    const selected_path = await open({
      directory: true,
      multiple: false,
      title: this.translate('decrypt.selectInputTitle'),
    })

    if (selected_path) {
      await this.saveInputPath(selected_path)
    }
  }

  async selectOutputFolder(): Promise<void> {
    const selected_path = await open({
      directory: true,
      multiple: false,
      title: this.translate('decrypt.selectOutputTitle'),
    })

    if (selected_path) {
      await this.saveOutputPath(selected_path)
    }
  }

  async saveInputPath(value: string): Promise<void> {
    this.input_path.value = value
    const store = await this.getStore()
    await store.set('decrypt-input-path', value)
  }

  handleInputPathChanged(value: string): void {
    void this.saveInputPath(value)
  }

  async saveOutputPath(value: string): Promise<void> {
    this.output_path.value = value
    const store = await this.getStore()
    await store.set('decrypt-output-path', value)
  }

  handleOutputPathChanged(value: string): void {
    void this.saveOutputPath(value)
  }

  async saveConcurrency(value: number): Promise<void> {
    this.concurrency.value = value
    const store = await this.getStore()
    await store.set('decrypt-concurrency', value)
  }

  handleSaveConcurrency(value: number): void {
    void this.saveConcurrency(value)
  }

  async saveSelectedGame(value: DecryptGameName): Promise<void> {
    this.selected_game.value = value
    const store = await this.getStore()
    await store.set('decrypt-game-name', value)
  }

  handleSaveSelectedGame(value: DecryptGameName): void {
    void this.saveSelectedGame(value)
  }

  async startDecryption(): Promise<void> {
    if (!this.validateInputPaths()) {
      return
    }

    this.is_decrypting.value = true

    try {
      await invoke(this.getDecryptCommandName(), {
        inputPath: this.input_path.value,
        outputPath: this.output_path.value,
        maxConcurrency: this.concurrency.value,
      })
    } catch (error) {
      this.addLog(this.translate('decrypt.errorDuringDecryption', { error }), 'error')
    } finally {
      this.is_decrypting.value = false
    }
  }

  handleGlobalStopSignal(): void {
    this.is_decrypting.value = false
  }

  private async getStore(): Promise<Awaited<ReturnType<typeof load>>> {
    if (!this.store) {
      this.store = await load('settings.json', { defaults: {}, autoSave: true })
    }

    return this.store
  }

  private validateInputPaths(): boolean {
    if (!this.input_path.value.trim()) {
      this.addLog(this.translate('decrypt.selectInputFirst'), 'error')
      return false
    }

    if (!this.output_path.value.trim()) {
      this.addLog(this.translate('decrypt.selectOutputFirst'), 'error')
      return false
    }

    if (this.isOutputPathUnsafe(this.input_path.value, this.output_path.value)) {
      this.addLog(this.translate('decrypt.sameDirectory'), 'error')
      return false
    }

    return true
  }

  private addLog(message: string, type: 'info' | 'success' | 'error' | 'warn' = 'info'): void {
    this.add_global_log(message, type)
  }

  private getDecryptCommandName(): string {
    switch (this.selected_game.value) {
      case DecryptGameName.NarakaBladepoint:
        return 'decrypt_naraka_bladepoint_files'
      case DecryptGameName.TheMagicBlade:
        return 'decrypt_the_magic_blade_files'
      case DecryptGameName.GirlsFrontline2:
      default:
        return 'decrypt_gf2_files'
    }
  }

  private isDecryptGameName(value: unknown): value is DecryptGameName {
    return (
      value === DecryptGameName.GirlsFrontline2 ||
      value === DecryptGameName.NarakaBladepoint ||
      value === DecryptGameName.TheMagicBlade
    )
  }

  private normalizePathForCompare(path: string): string {
    return path.trim().replace(/\//g, '\\').replace(/\\+$/g, '').toLowerCase()
  }

  private isOutputPathUnsafe(inputPath: string, outputPath: string): boolean {
    const input = this.normalizePathForCompare(inputPath)
    const output = this.normalizePathForCompare(outputPath)
    return output === input || output.startsWith(`${input}\\`)
  }
}
