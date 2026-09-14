<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { Moon, Sunny, Cpu, Coin, FolderOpened, CopyDocument, Link, Promotion, ChatDotRound, Refresh } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { useTheme } from '../composables/useTheme'
import { SettingsPageController } from './SettingsPageController'
import { UpdateChecker, isCheckingUpdate } from '../utils/UpdateChecker'
import appIconUrl from '../../src-tauri/icons/64x64.png'

const { theme, colorScheme, colorSchemeOptions, loadTheme, setTheme, setColorScheme } = useTheme()
const { t, locale } = useI18n()
const settings_page_controller = new SettingsPageController({
  locale,
  loadTheme,
  setTheme,
  setColorScheme,
})
const app_version = ref('...')
const github_url = 'https://github.com/StarBobis/AssetDaemon'
const issue_url = 'https://github.com/StarBobis/AssetDaemon/issues'
const qq_group = '574100817'

onMounted(async () => {
  await settings_page_controller.loadSettings()
  app_version.value = await getVersion().catch(() => 'unknown')
})
</script>

<template>
  <div class="page">
    <div class="settings-layout">
      <main class="settings-main">
        <section class="settings-section">
          <div class="section-title">
            <span>{{ t('settings.appearance') }}</span>
            <small>{{ t('settings.userInterface') }}</small>
          </div>

          <div class="setting-row">
            <div class="setting-label">
              <span>{{ t('settings.themeMode') }}</span>
              <span class="setting-desc">{{ t('settings.themeDescription') }}</span>
            </div>
            <div class="setting-control">
              <el-segmented
                :model-value="theme"
                :options="[
                  { label: t('settings.dark'), value: 'dark', icon: Moon },
                  { label: t('settings.light'), value: 'light', icon: Sunny },
                ]"
                @update:model-value="settings_page_controller.handleThemeChange"
              />
            </div>
          </div>

          <div class="setting-row">
            <div class="setting-label">
              <span>{{ t('settings.colorScheme') }}</span>
              <span class="setting-desc">{{ t('settings.colorSchemeDescription') }}</span>
            </div>
            <div class="setting-control scheme-control">
              <el-select
                :model-value="colorScheme"
                :placeholder="t('settings.colorScheme')"
                @update:model-value="settings_page_controller.handleColorSchemeChange"
              >
                <el-option
                  v-for="option in colorSchemeOptions"
                  :key="option.value"
                  :label="t(option.labelKey)"
                  :value="option.value"
                >
                  <div class="scheme-option">
                    <span class="scheme-copy">
                      <strong>{{ t(option.labelKey) }}</strong>
                      <small>{{ t(option.descriptionKey) }}</small>
                    </span>
                    <span class="scheme-swatches">
                      <i
                        v-for="color in option.colors"
                        :key="color"
                        :style="{ background: color }"
                      ></i>
                    </span>
                  </div>
                </el-option>
              </el-select>
            </div>
          </div>

          <div class="setting-row">
            <div class="setting-label">
              <span>{{ t('settings.language') }}</span>
              <span class="setting-desc">{{ t('settings.languageDescription') }}</span>
            </div>
            <div class="setting-control">
              <el-segmented
                :model-value="settings_page_controller.selected_language.value"
                :options="[
                  { label: t('settings.english'), value: 'en' },
                  { label: t('settings.chinese'), value: 'zh' },
                ]"
                @update:model-value="settings_page_controller.setLanguage"
              />
            </div>
          </div>
        </section>

        <section class="settings-section">
          <div class="section-title">
            <span>{{ t('settings.bundleTypeScan') }}</span>
            <small>{{ t('settings.parsingPerformance') }}</small>
          </div>

          <div class="setting-row">
            <div class="setting-label">
              <span>{{ t('settings.scanConcurrency') }}</span>
              <span class="setting-desc">{{ t('settings.scanConcurrencyDescription') }}</span>
            </div>
            <div class="setting-control wide-control">
              <el-slider
                v-model="settings_page_controller.concurrency.value"
                :min="1"
                :max="32"
                :step="1"
                show-input
                @change="settings_page_controller.setConcurrency"
              />
            </div>
          </div>

          <div class="setting-row">
            <div class="setting-label">
              <span>{{ t('settings.typeCache') }}</span>
              <span class="setting-desc">{{ t('settings.typeCacheDescription') }}</span>
            </div>
            <div class="setting-control">
              <el-tag type="success" effect="plain">
                <el-icon><Cpu /></el-icon>
                {{ t('settings.autoCache') }}
              </el-tag>
            </div>
          </div>
        </section>

        <section class="settings-section">
          <div class="section-title">
            <span>{{ t('settings.cacheLocation') }}</span>
            <small>{{ t('settings.assetMapStorage') }}</small>
          </div>

          <div class="setting-row">
            <div class="setting-label">
              <span>{{ t('settings.assetMapCacheRoot') }}</span>
              <span class="setting-desc">
                {{ t('settings.assetMapCacheDescription') }}
              </span>
            </div>
            <div class="setting-control path-control">
              <el-input
                :model-value="settings_page_controller.asset_map_cache_root.value"
                readonly
              />
              <el-button
                :title="t('settings.selectCacheFolder')"
                @click="settings_page_controller.selectAssetMapCacheRoot"
              >
                <el-icon><FolderOpened /></el-icon>
              </el-button>
              <el-button
                :title="t('settings.copyCachePath')"
                @click="settings_page_controller.copyAssetMapCacheRoot"
              >
                <el-icon><CopyDocument /></el-icon>
              </el-button>
            </div>
          </div>

          <div class="cache-note">
            <el-icon><Coin /></el-icon>
            <span>{{ t('settings.assetMapCacheNote') }}</span>
          </div>
        </section>

        <section class="settings-section">
          <div class="section-title">
            <span>{{ t('settings.bundleParseCache') }}</span>
            <small>{{ t('settings.memoryBudget') }}</small>
          </div>

          <div class="setting-row">
            <div class="setting-label">
              <span>{{ t('settings.memoryCacheLimit') }}</span>
              <span class="setting-desc">
                {{ t('settings.memoryCacheDescription') }}
              </span>
            </div>
            <div class="setting-control wide-control">
              <el-slider
                v-model="settings_page_controller.cache_max_gb.value"
                :min="1"
                :max="48"
                :step="1"
                show-input
                @change="settings_page_controller.setCacheMaxGB"
              />
            </div>
          </div>

          <div class="cache-note">
            <el-icon><Coin /></el-icon>
            <span>{{ t('settings.cacheNote') }}</span>
          </div>
        </section>
      </main>

      <aside class="settings-side">
        <section class="about-card">
          <div class="about-card-hero">
            <div class="about-logo">
              <img :src="appIconUrl" alt="" aria-hidden="true" />
            </div>
            <div>
              <h2>{{ t('settings.appName') }}</h2>
              <p>{{ t('settings.appDescription') }}</p>
            </div>
          </div>

          <div class="about-list">
            <div class="about-row">
              <span>{{ t('settings.version') }}</span>
              <strong>{{ app_version }}</strong>
            </div>
            <div class="about-row">
              <span>{{ t('settings.github') }}</span>
              <a :href="github_url" target="_blank" rel="noreferrer">{{ github_url }}</a>
            </div>
            <div class="about-row">
              <span>{{ t('settings.submitIssue') }}</span>
              <a :href="issue_url" target="_blank" rel="noreferrer">{{ issue_url }}</a>
            </div>
            <div class="about-row">
              <span>{{ t('settings.qqGroup') }}</span>
              <strong>{{ qq_group }}</strong>
            </div>
          </div>

          <div class="about-actions">
            <el-button :loading="isCheckingUpdate" @click="UpdateChecker.checkForUpdates()">
              <el-icon><Refresh /></el-icon>
              {{ t('settings.checkUpdates') }}
            </el-button>
            <el-button tag="a" :href="github_url" target="_blank" rel="noreferrer">
              <el-icon><Link /></el-icon>
              {{ t('settings.openGithub') }}
            </el-button>
            <el-button tag="a" type="primary" :href="issue_url" target="_blank" rel="noreferrer">
              <el-icon><Promotion /></el-icon>
              {{ t('settings.openIssue') }}
            </el-button>
          </div>

          <div class="github-token-box">
            <label>{{ t('settings.githubToken') }}</label>
            <el-input
              :model-value="settings_page_controller.github_token.value"
              type="password"
              show-password
              clearable
              :placeholder="t('settings.githubTokenPlaceholder')"
              @update:model-value="(value: string | number) => settings_page_controller.setGithubToken(String(value))"
            />
            <p>{{ t('settings.githubTokenHint') }}</p>
          </div>

          <div class="about-note">
            <el-icon><ChatDotRound /></el-icon>
            <span>{{ t('settings.supportHint', { qq: qq_group }) }}</span>
          </div>
        </section>
      </aside>
    </div>
  </div>
</template>

<style scoped>
.page {
  width: 100%;
  height: 100%;
  overflow: hidden;
  padding: 14px;
  background: var(--app-bg);
}

.settings-layout {
  display: grid;
  grid-template-columns: minmax(560px, 1fr) 360px;
  gap: 16px;
  align-items: stretch;
  height: 100%;
  min-height: 0;
}

.settings-main {
  min-width: 0;
  min-height: 0;
  overflow: auto;
  padding-right: 2px;
}

.settings-side {
  min-width: 0;
  min-height: 0;
  overflow: auto;
  padding-right: 2px;
}

.settings-section {
  margin-bottom: 12px;
  border: 1px solid var(--app-border-soft);
  border-radius: 6px;
  background: var(--app-surface);
  box-shadow: var(--app-shadow-sm);
  overflow: hidden;
}

.section-title {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--app-border-soft);
  background: var(--app-surface-soft);
}

.section-title span {
  font-size: 12px;
  font-weight: 700;
  text-transform: uppercase;
  color: var(--app-text-primary);
}

.section-title small {
  font-size: 12px;
  color: var(--app-text-secondary);
}

.setting-row {
  display: grid;
  grid-template-columns: minmax(260px, 1fr) minmax(320px, 42%);
  align-items: center;
  gap: 18px;
  padding: 14px;
  border-bottom: 1px solid var(--app-border-soft);
}

.setting-row:last-child {
  border-bottom: 0;
}

.setting-label {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 14px;
  font-weight: 600;
  min-width: 0;
}

.setting-desc {
  font-size: 12px;
  font-weight: 400;
  color: var(--el-text-color-secondary);
  line-height: 1.5;
}

.setting-control {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  min-width: 0;
}

.setting-control :deep(.el-segmented) {
  width: 220px;
}

.scheme-control :deep(.el-select) {
  width: 260px;
}

.scheme-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-width: 0;
}

.scheme-copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.scheme-copy strong {
  color: var(--app-text-primary);
  font-size: 13px;
  line-height: 1.2;
}

.scheme-copy small {
  color: var(--app-text-secondary);
  font-size: 11px;
  line-height: 1.2;
}

.scheme-swatches {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: 0 0 auto;
}

.scheme-swatches i {
  width: 14px;
  height: 14px;
  border: 1px solid color-mix(in srgb, var(--app-text-primary) 18%, transparent);
  border-radius: 50%;
}

.wide-control {
  width: 100%;
  justify-content: stretch;
}

.path-control {
  width: 100%;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  gap: 8px;
}

.path-control :deep(.el-button + .el-button) {
  margin-left: 0;
}

.cache-note {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  color: var(--app-text-secondary);
  background: var(--app-surface-soft);
  font-size: 12px;
  line-height: 1.5;
}

.cache-note .el-icon {
  color: var(--el-color-primary);
  flex-shrink: 0;
}

.about-card {
  overflow: hidden;
  border: 1px solid var(--app-border-soft);
  border-radius: 6px;
  background: var(--app-surface);
  box-shadow: var(--app-shadow-sm);
}

.about-card-hero {
  display: flex;
  gap: 12px;
  padding: 14px;
  border-bottom: 1px solid var(--app-border-soft);
  background: var(--app-surface-soft);
}

.about-logo {
  width: 40px;
  height: 40px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  border: 1px solid var(--app-border-soft);
  border-radius: 8px;
  background: var(--app-surface);
}

.about-logo img {
  width: 26px;
  height: 26px;
  display: block;
  object-fit: contain;
}

.about-card h2 {
  margin: 0;
  color: var(--app-text-primary);
  font-size: 16px;
  font-weight: 700;
}

.about-card p {
  margin: 6px 0 0;
  color: var(--app-text-secondary);
  font-size: 12px;
  line-height: 1.5;
}

.about-list {
  display: flex;
  flex-direction: column;
  padding: 8px 14px;
}

.about-row {
  display: grid;
  grid-template-columns: 88px minmax(0, 1fr);
  gap: 12px;
  padding: 8px 0;
  border-bottom: 1px solid var(--app-border-soft);
  font-size: 12px;
}

.about-row:last-child {
  border-bottom: 0;
}

.about-row span {
  color: var(--app-text-secondary);
}

.about-row strong,
.about-row a {
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--app-text-primary);
  font-weight: 650;
}

.about-row a:hover {
  color: var(--el-color-primary);
}

.about-actions {
  display: grid;
  grid-template-columns: 1fr;
  gap: 8px;
  padding: 0 14px 14px;
}

.about-actions :deep(.el-button) {
  width: 100%;
  margin-left: 0;
}

.github-token-box {
  display: flex;
  flex-direction: column;
  gap: 7px;
  padding: 0 14px 14px;
}

.github-token-box label {
  color: var(--app-text-primary);
  font-size: 12px;
  font-weight: 750;
}

.github-token-box p {
  margin: 0;
  color: var(--app-text-secondary);
  font-size: 11px;
  line-height: 1.45;
}

.about-note {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 12px 14px;
  border-top: 1px solid var(--app-border-soft);
  background: var(--app-surface-soft);
  color: var(--app-text-secondary);
  font-size: 12px;
  line-height: 1.5;
}

.about-note .el-icon {
  margin-top: 2px;
  color: var(--el-color-warning);
  flex-shrink: 0;
}

@media (max-width: 760px) {
  .page {
    padding: 16px;
  }

  .setting-row {
    grid-template-columns: 1fr;
    align-items: stretch;
  }

  .setting-control,
  .wide-control {
    width: 100%;
    justify-content: stretch;
  }

  .setting-control :deep(.el-segmented) {
    width: 100%;
  }

  .path-control {
    width: 100%;
    grid-template-columns: 1fr auto auto;
  }
}

@media (max-width: 1080px) {
  .page {
    overflow: auto;
  }

  .settings-layout {
    grid-template-columns: 1fr;
    height: auto;
  }

  .settings-main,
  .settings-side {
    overflow: visible;
    padding-right: 0;
  }
}
</style>
