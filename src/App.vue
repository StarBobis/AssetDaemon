<script setup lang="ts">
/**
 * Root application component.
 *
 * Owns global chrome: title bar, routed page content, and shared log panel.
 */

import { onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { listen } from '@tauri-apps/api/event'
import { ElMessageBox } from 'element-plus'
import ContentArea from './components/ContentArea.vue'
import CustomTitlebar from './components/CustomTitlebar.vue'
import { useGlobalLogLayout } from './composables/useGlobalLogLayout'
import { useLogSystem } from './composables/useLogSystem'
import { useTheme } from './composables/useTheme'
import { I18nLanguageUtils } from './i18n'
import { WindowUtils } from './utils/WindowUtils'

/** Payload emitted by the Rust backend when a background task dies from a panic. */
interface PanicAlertPayload {
  task_label: string
  message: string
  log_path: string
}

const logLayout = useGlobalLogLayout()
const { registerBackendLogChannel, addLog } = useLogSystem()
const { loadTheme } = useTheme()
const { t, locale } = useI18n()

let unlistenPanicAlert: (() => void) | null = null
let panicAlertShowing = false

onMounted(() => {
  void I18nLanguageUtils.loadPersistedLocale(locale)
  void WindowUtils.initializeWindowPersistence()
  void loadTheme()
  document.addEventListener('contextmenu', (e) => e.preventDefault())
  logLayout.loadLogLayout()
  registerBackendLogChannel()

  // Global panic fallback: the backend has already written the panic log to
  // disk and revealed it in the file manager; pop a dialog so the user knows
  // a crash happened and where to find the report.
  void listen<PanicAlertPayload>('panic-alert', (event) => {
    const payload = event.payload
    addLog(
      `[Panic] ${payload.task_label}: ${payload.message} (log: ${payload.log_path})`,
      'error',
    )
    if (panicAlertShowing) return
    panicAlertShowing = true
    void ElMessageBox.alert(
      t('panic.alertMessage', {
        task: payload.task_label,
        message: payload.message,
        logPath: payload.log_path,
      }),
      t('panic.alertTitle'),
      { type: 'error', confirmButtonText: t('common.confirm') },
    ).finally(() => {
      panicAlertShowing = false
    })
  }).then((unlisten) => {
    unlistenPanicAlert = unlisten
  })
})

onUnmounted(() => {
  unlistenPanicAlert?.()
  unlistenPanicAlert = null
})
</script>

<template>
  <div class="app-shell">
    <CustomTitlebar />
    <ContentArea />
  </div>
</template>

<style>
*, *::before, *::after { margin: 0; padding: 0; box-sizing: border-box; }

html, body, #app {
  height: 100%;
  overflow: hidden;
}

:root {
  font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 24px;
  font-weight: 400;
  color: var(--app-text-primary);
  background-color: var(--app-bg);
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  -webkit-text-size-adjust: 100%;
  --app-color-lime: #6ECC54;
  --app-color-titian: #D34947;
  --app-color-mars: #018B8D;
  --app-color-klein: #002FA7;
  --app-color-burgundy: #470125;
  --app-color-schonbrunn: #F9D46C;
  --app-color-tiffany: #71E2D1;
  --app-color-china-red: #C8161D;
  --app-color-vandyke: #492D22;
  --app-color-hermes: #EB5C20;
  --app-color-prussian: #0D3A69;
  --app-accent-primary: var(--app-color-mars);
  --app-accent-secondary: var(--app-color-tiffany);
  --app-accent-strong: var(--app-color-klein);
  --app-accent-warm: var(--app-color-hermes);
  --app-accent-highlight: var(--app-color-schonbrunn);
  --app-accent-deep: var(--app-color-prussian);
  --app-bg: #edf5f5;
  --app-surface: #ffffff;
  --app-surface-soft: #f5faf9;
  --app-surface-muted: #e8f1f0;
  --app-border: #cddddd;
  --app-border-soft: #dceaea;
  --app-text-primary: #102029;
  --app-text-secondary: #52686c;
  --app-shadow-sm: 0 1px 2px rgba(13, 58, 105, 0.06);
  --app-shadow-md: 0 12px 30px rgba(13, 58, 105, 0.11);
  --app-shadow-lg: 0 24px 70px rgba(13, 58, 105, 0.18);
  --el-color-primary: var(--app-accent-primary);
  --el-color-primary-light-3: color-mix(in srgb, var(--app-accent-primary) 70%, white);
  --el-color-primary-light-5: color-mix(in srgb, var(--app-accent-primary) 50%, white);
  --el-color-primary-light-7: color-mix(in srgb, var(--app-accent-primary) 30%, white);
  --el-color-primary-light-8: color-mix(in srgb, var(--app-accent-primary) 20%, white);
  --el-color-primary-light-9: color-mix(in srgb, var(--app-accent-primary) 10%, white);
  --el-color-primary-dark-2: color-mix(in srgb, var(--app-accent-primary) 78%, black);
  --el-color-success: var(--app-color-lime);
  --el-color-warning: var(--app-accent-warm);
  --el-color-danger: var(--app-color-titian);
  --el-color-info: var(--app-accent-deep);
  --el-color-success-light-9: #effaea;
  --el-color-warning-light-5: #f5ad8f;
  --el-color-warning-light-9: #fdf0e9;
  --el-color-danger-light-9: #fbecec;
  --el-border-radius-base: 6px;
  --el-border-color: var(--app-border);
  --el-border-color-light: var(--app-border);
  --el-border-color-lighter: var(--app-border-soft);
  --el-bg-color: var(--app-surface);
  --el-bg-color-page: var(--app-bg);
  --el-fill-color-light: var(--app-surface-soft);
  --el-fill-color-lighter: var(--app-surface-muted);
  --el-text-color-primary: var(--app-text-primary);
  --el-text-color-regular: #263940;
  --el-text-color-secondary: var(--app-text-secondary);
  --app-selection-bg: color-mix(in srgb, var(--app-accent-primary) 45%, transparent);
  --app-selection-color: #ffffff;
  --app-selection-glow: color-mix(in srgb, var(--app-accent-primary) 50%, transparent);
}

:root[data-theme='dark'] {
  --app-bg: #07090b;
  --app-surface: #101316;
  --app-surface-soft: #151a1e;
  --app-surface-muted: #1d2429;
  --app-border: #323b42;
  --app-border-soft: #252d33;
  --app-text-primary: #e8edf0;
  --app-text-secondary: #9aa8ae;
  --app-shadow-sm: 0 1px 2px rgba(0, 0, 0, 0.38);
  --app-shadow-md: 0 16px 38px rgba(0, 0, 0, 0.46);
  --app-shadow-lg: 0 28px 80px rgba(0, 0, 0, 0.55);
  --el-color-primary: var(--app-accent-primary);
  --el-color-primary-light-3: color-mix(in srgb, var(--app-accent-primary) 68%, white);
  --el-color-primary-light-5: color-mix(in srgb, var(--app-accent-primary) 46%, var(--app-surface));
  --el-color-primary-light-7: color-mix(in srgb, var(--app-accent-primary) 32%, var(--app-surface));
  --el-color-primary-light-8: color-mix(in srgb, var(--app-accent-primary) 24%, var(--app-surface));
  --el-color-primary-light-9: color-mix(in srgb, var(--app-accent-primary) 16%, var(--app-surface));
  --el-color-primary-dark-2: color-mix(in srgb, var(--app-accent-primary) 70%, white);
  --el-color-success: var(--app-color-lime);
  --el-color-warning: var(--app-accent-highlight);
  --el-color-danger: var(--app-color-titian);
  --el-color-info: color-mix(in srgb, var(--app-accent-deep) 46%, white);
  --el-color-success-light-9: #1f3b2a;
  --el-color-warning-light-5: #756535;
  --el-color-warning-light-9: #332d1b;
  --el-color-danger-light-9: #351e22;
  --el-border-color: var(--app-border);
  --el-border-color-light: var(--app-border);
  --el-border-color-lighter: var(--app-border-soft);
  --el-border-color-extra-light: #1d252b;
  --el-bg-color: var(--app-surface);
  --el-bg-color-page: var(--app-bg);
  --el-fill-color: #171d21;
  --el-fill-color-light: var(--app-surface-soft);
  --el-fill-color-lighter: var(--app-surface-muted);
  --el-fill-color-darker: #050607;
  --el-text-color-primary: var(--app-text-primary);
  --el-text-color-regular: #d0dfdc;
  --el-text-color-secondary: var(--app-text-secondary);
  --el-text-color-placeholder: #6f6f6f;
  --el-text-color-disabled: #555555;
  --app-selection-bg: color-mix(in srgb, var(--app-accent-secondary) 22%, transparent);
  --app-selection-color: var(--app-accent-secondary);
  --app-selection-glow: color-mix(in srgb, var(--app-accent-secondary) 40%, transparent);
}


:root[data-color-scheme='lime'] {
  --app-accent-primary: var(--app-color-lime);
  --app-accent-secondary: var(--app-color-mars);
  --app-accent-strong: var(--app-color-prussian);
  --app-accent-warm: var(--app-color-schonbrunn);
  --app-accent-highlight: var(--app-color-lime);
  --app-accent-deep: var(--app-color-prussian);
  --app-bg: #f2f8ef;
  --app-surface-soft: #f7fbf4;
  --app-surface-muted: #e8f2e3;
  --app-border: #cfdfc7;
  --app-border-soft: #dcead6;
  --app-text-secondary: #58705b;
}

:root[data-color-scheme='titian'] {
  --app-accent-primary: var(--app-color-titian);
  --app-accent-secondary: var(--app-color-china-red);
  --app-accent-strong: var(--app-color-burgundy);
  --app-accent-warm: var(--app-color-schonbrunn);
  --app-accent-highlight: var(--app-color-titian);
  --app-accent-deep: var(--app-color-burgundy);
  --app-bg: #f8f0f0;
  --app-surface-soft: #fbf5f5;
  --app-surface-muted: #f0e3e3;
  --app-border: #dfcccc;
  --app-border-soft: #eadada;
  --app-text-secondary: #705459;
}

:root[data-color-scheme='mars'] {
  --app-accent-primary: var(--app-color-mars);
  --app-accent-secondary: var(--app-color-tiffany);
  --app-accent-strong: var(--app-color-klein);
  --app-accent-warm: var(--app-color-hermes);
  --app-accent-highlight: var(--app-color-schonbrunn);
  --app-accent-deep: var(--app-color-prussian);
}

:root[data-color-scheme='klein'] {
  --app-accent-primary: var(--app-color-klein);
  --app-accent-secondary: var(--app-color-tiffany);
  --app-accent-strong: var(--app-color-prussian);
  --app-accent-warm: var(--app-color-schonbrunn);
  --app-accent-highlight: var(--app-color-klein);
  --app-accent-deep: var(--app-color-prussian);
  --app-bg: #eef2fb;
  --app-surface-soft: #f4f7fd;
  --app-surface-muted: #e4eafa;
  --app-border: #cad4ea;
  --app-border-soft: #dae2f3;
  --app-text-secondary: #53637e;
}

:root[data-color-scheme='burgundy'] {
  --app-accent-primary: var(--app-color-burgundy);
  --app-accent-secondary: var(--app-color-titian);
  --app-accent-strong: var(--app-color-china-red);
  --app-accent-warm: var(--app-color-schonbrunn);
  --app-accent-highlight: var(--app-color-titian);
  --app-accent-deep: var(--app-color-vandyke);
  --app-bg: #f6eff2;
  --app-surface-soft: #fbf4f6;
  --app-surface-muted: #efe2e7;
  --app-border: #ddcbd2;
  --app-border-soft: #eadbe0;
  --app-text-primary: #25131c;
  --app-text-secondary: #6a5260;
}

:root[data-color-scheme='schonbrunn'] {
  --app-accent-primary: var(--app-color-schonbrunn);
  --app-accent-secondary: var(--app-color-hermes);
  --app-accent-strong: var(--app-color-mars);
  --app-accent-warm: var(--app-color-hermes);
  --app-accent-highlight: var(--app-color-schonbrunn);
  --app-accent-deep: var(--app-color-vandyke);
  --app-bg: #f8f4e7;
  --app-surface-soft: #fcf8ec;
  --app-surface-muted: #f0e8d2;
  --app-border: #ded2b6;
  --app-border-soft: #eadfc5;
  --app-text-primary: #2c2518;
  --app-text-secondary: #6b6047;
}

:root[data-color-scheme='tiffany'] {
  --app-accent-primary: var(--app-color-tiffany);
  --app-accent-secondary: var(--app-color-mars);
  --app-accent-strong: var(--app-color-klein);
  --app-accent-warm: var(--app-color-schonbrunn);
  --app-accent-highlight: var(--app-color-tiffany);
  --app-accent-deep: var(--app-color-prussian);
  --app-bg: #eef9f7;
  --app-surface-soft: #f3fbf9;
  --app-surface-muted: #e3f3f0;
  --app-border: #c8e1dd;
  --app-border-soft: #d8ece8;
  --app-text-secondary: #4f6965;
}

:root[data-color-scheme='china-red'] {
  --app-accent-primary: var(--app-color-china-red);
  --app-accent-secondary: var(--app-color-titian);
  --app-accent-strong: var(--app-color-burgundy);
  --app-accent-warm: var(--app-color-schonbrunn);
  --app-accent-highlight: var(--app-color-china-red);
  --app-accent-deep: var(--app-color-burgundy);
  --app-bg: #f7eff0;
  --app-surface-soft: #fbf4f4;
  --app-surface-muted: #efe1e2;
  --app-border: #dec9cc;
  --app-border-soft: #ead8da;
  --app-text-secondary: #704f56;
}

:root[data-color-scheme='vandyke'] {
  --app-accent-primary: var(--app-color-vandyke);
  --app-accent-secondary: var(--app-color-hermes);
  --app-accent-strong: var(--app-color-prussian);
  --app-accent-warm: var(--app-color-schonbrunn);
  --app-accent-highlight: var(--app-color-hermes);
  --app-accent-deep: var(--app-color-vandyke);
  --app-bg: #f4f0ed;
  --app-surface-soft: #faf6f2;
  --app-surface-muted: #e9dfd8;
  --app-border: #d8c8bd;
  --app-border-soft: #e6d8cf;
  --app-text-primary: #281d18;
  --app-text-secondary: #66584f;
}

:root[data-color-scheme='hermes'] {
  --app-accent-primary: var(--app-color-hermes);
  --app-accent-secondary: var(--app-color-schonbrunn);
  --app-accent-strong: var(--app-color-mars);
  --app-accent-warm: var(--app-color-schonbrunn);
  --app-accent-highlight: var(--app-color-hermes);
  --app-accent-deep: var(--app-color-vandyke);
  --app-bg: #f7f2ea;
  --app-surface-soft: #fbf6ed;
  --app-surface-muted: #efe4d6;
  --app-border: #ded0bf;
  --app-border-soft: #eadfce;
  --app-text-primary: #281d18;
  --app-text-secondary: #69584d;
}

:root[data-color-scheme='prussian'] {
  --app-accent-primary: var(--app-color-prussian);
  --app-accent-secondary: var(--app-color-mars);
  --app-accent-strong: var(--app-color-klein);
  --app-accent-warm: var(--app-color-hermes);
  --app-accent-highlight: var(--app-color-tiffany);
  --app-accent-deep: var(--app-color-prussian);
  --app-bg: #edf2f5;
  --app-surface-soft: #f5f8fa;
  --app-surface-muted: #e5edf2;
  --app-border: #cbd7df;
  --app-border-soft: #dce6ec;
  --app-text-secondary: #536674;
}

:root[data-theme='dark'] {
  --app-bg: #07090b;
  --app-surface: #101316;
  --app-surface-soft: #151a1e;
  --app-surface-muted: #1d2429;
  --app-border: #323b42;
  --app-border-soft: #252d33;
  --app-text-primary: #e8edf0;
  --app-text-secondary: #9aa8ae;
}


::selection {
  background: var(--app-selection-bg);
  color: var(--app-selection-color);
  text-shadow: 0 0 6px var(--app-selection-glow);
}

::-moz-selection {
  background: var(--app-selection-bg);
  color: var(--app-selection-color);
  text-shadow: 0 0 6px var(--app-selection-glow);
}

body {
  color: var(--app-text-primary);
  background: var(--app-bg);
}

button,
input,
textarea,
select {
  font: inherit;
}

.app-shell {
  height: 100vh;
  display: grid;
  grid-template-rows: 34px minmax(0, 1fr);
  overflow: hidden;
}

.el-button {
  font-weight: 600;
}

.el-card {
  border-color: var(--app-border-soft);
  background: var(--app-surface);
  box-shadow: var(--app-shadow-sm);
}

.el-message {
  z-index: 10020 !important;
}

::-webkit-scrollbar {
  width: 10px;
  height: 10px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background: color-mix(in srgb, var(--app-text-secondary) 24%, transparent);
  border: 3px solid transparent;
  border-radius: 999px;
  background-clip: content-box;
}

::-webkit-scrollbar-thumb:hover {
  background: color-mix(in srgb, var(--app-text-secondary) 38%, transparent);
  border: 3px solid transparent;
  background-clip: content-box;
}
</style>
