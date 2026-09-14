<script setup lang="ts">
import { useRouter, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import {
  Minus,
  FullScreen,
  Close,
  Setting,
  Promotion,
  Unlock,
  WarnTriangleFilled,
  UploadFilled,
} from '@element-plus/icons-vue'
import appIconUrl from '../../src-tauri/icons/32x32.png'
import { CustomTitlebarController } from './CustomTitlebarController'
import { UpdateChecker, isCheckingUpdate } from '../utils/UpdateChecker'

const router = useRouter()
const route = useRoute()
const { t } = useI18n()
const custom_titlebar_controller = new CustomTitlebarController({
  router,
  route,
})
</script>

<template>
  <div class="custom-titlebar">
    <div class="drag-area">
      <button class="brand" :title="t('titlebar.workspace')" @click="custom_titlebar_controller.navigate('/')">
        <img class="brand-icon" :src="appIconUrl" alt="" aria-hidden="true" />
        <span>AssetDaemon</span>
      </button>
    </div>
    <div class="titlebar-right">
      <el-popover
        trigger="hover"
        placement="bottom-end"
        popper-class="disclaimer-popover"
        :width="420"
        :show-after="120"
        :hide-after="80"
      >
        <template #reference>
          <button class="win-btn disclaimer-btn" :title="t('titlebar.disclaimer')">
            <el-icon><WarnTriangleFilled /></el-icon>
          </button>
        </template>
        <div class="disclaimer-card">
          <div class="disclaimer-card__title">
            <el-icon><WarnTriangleFilled /></el-icon>
            <span>{{ t('disclaimer.title') }}</span>
          </div>
          <div class="disclaimer-card__scroll">
            <p
              v-for="item in t('disclaimer.items').split('\n')"
              :key="item"
              class="disclaimer-card__text"
            >
              {{ item }}
            </p>
          </div>
          <strong class="disclaimer-card__danger">
            {{ t('disclaimer.finalWarning') }}
          </strong>
        </div>
      </el-popover>
      <button class="win-btn update-btn" :title="t('titlebar.checkUpdates')" @click="UpdateChecker.checkForUpdates()">
        <el-icon class="update-btn__icon" :class="{ 'is-loading': isCheckingUpdate }"><UploadFilled /></el-icon>
      </button>
      <button class="win-btn issue-btn" :title="t('titlebar.reportIssue')" @click="custom_titlebar_controller.openIssuePage">
        <el-icon><Promotion /></el-icon>
      </button>
      <button class="win-btn" :title="t('titlebar.decrypt')" @click="custom_titlebar_controller.toggleTemporaryPage('/decrypt')">
        <el-icon><Unlock /></el-icon>
      </button>
      <button class="win-btn" :title="t('titlebar.settings')" @click="custom_titlebar_controller.toggleTemporaryPage('/settings')">
        <el-icon><Setting /></el-icon>
      </button>
      <span class="separator"></span>
      <div class="window-controls">
        <button class="win-btn" :title="t('titlebar.minimize')" @click="custom_titlebar_controller.minimize">
          <el-icon><Minus /></el-icon>
        </button>
        <button
          class="win-btn"
          :title="custom_titlebar_controller.is_maximized.value ? t('titlebar.restore') : t('titlebar.maximize')"
          @click="custom_titlebar_controller.toggleMaximize"
        >
          <el-icon>
            <FullScreen v-if="!custom_titlebar_controller.is_maximized.value" />
            <svg v-else viewBox="0 0 24 24" width="1em" height="1em" fill="currentColor">
              <path d="M4 8h8V4H2v10h2V8zm2 2v10h10V10H6zm2 2h6v6H8v-6z"/>
              <path d="M14 2v2h4v4h2V2h-6z" opacity="0.4"/>
            </svg>
          </el-icon>
        </button>
        <button class="win-btn" :title="t('titlebar.close')" @click="custom_titlebar_controller.closeWindow">
          <el-icon><Close /></el-icon>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.custom-titlebar {
  -webkit-app-region: drag;
  user-select: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 34px;
  padding: 0 4px 0 10px;
  background: var(--app-surface-soft);
  color: var(--el-text-color-primary);
  border-bottom: 1px solid var(--app-border);
  position: relative;
  z-index: 10;
}

.drag-area { display: flex; align-items: center; min-width: 0; }

.brand {
  -webkit-app-region: no-drag;
  appearance: none;
  border: 0;
  background: transparent;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 28px;
  padding: 0 12px 0 4px;
  margin-right: 8px;
  color: var(--app-text-primary);
  font-size: 13px;
  font-weight: 750;
  letter-spacing: 0;
  border-right: 1px solid var(--app-border-soft);
}

.brand:hover {
  color: var(--app-text-primary);
  background: color-mix(in srgb, var(--app-text-secondary) 10%, transparent);
}

.brand-icon {
  width: 16px;
  height: 16px;
  display: block;
  object-fit: contain;
}

.window-controls { display: flex; align-items: center; }

.titlebar-right {
  display: flex;
  align-items: center;
  gap: 0;
}

.separator {
  width: 1px;
  height: 16px;
  background: var(--el-border-color-light);
  margin: 0 4px;
}

.win-btn {
  -webkit-app-region: no-drag;
  appearance: none;
  border: none;
  outline: none;
  background: transparent;
  cursor: pointer;
  width: 36px;
  height: 28px;
  padding: 0;
  font-size: 14px;
  border-radius: 4px;
  color: var(--el-text-color-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.win-btn :deep(.el-icon) { margin: 0; }
.win-btn:hover {
  color: var(--el-text-color-primary);
  background: var(--app-surface-soft);
}

.issue-btn {
  color: var(--el-color-primary);
}

.issue-btn:hover {
  color: #fff;
  background: var(--el-color-primary);
}

.update-btn {
  color: var(--app-color-tiffany);
}

.update-btn__icon {
  font-size: 15px;
}

.update-btn:hover {
  color: #fff;
  background: var(--app-color-mars);
}

.disclaimer-btn {
  color: var(--app-color-hermes);
}

.disclaimer-btn:hover {
  color: #fff;
  background: var(--app-color-hermes);
}

.disclaimer-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  max-height: min(520px, 72vh);
  padding: 2px;
  color: var(--el-text-color-primary);
}

.disclaimer-card__scroll {
  display: flex;
  flex: 1 1 auto;
  min-height: 0;
  flex-direction: column;
  gap: 8px;
  overflow-x: hidden;
  overflow-y: scroll;
  padding-right: 4px;
  scrollbar-gutter: stable;
}

.disclaimer-card__title {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 8px;
  padding-bottom: 6px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  color: var(--app-color-hermes);
  font-size: 14px;
  font-weight: 800;
}

.disclaimer-card__text {
  margin: 0;
  color: var(--el-text-color-regular);
  font-size: 12px;
  line-height: 1.55;
}

.disclaimer-card__danger {
  flex: 0 0 auto;
  margin-top: 4px;
  padding: 8px 10px;
  border: 1px solid color-mix(in srgb, var(--el-color-danger) 42%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--el-color-danger) 10%, transparent);
  color: var(--el-color-danger);
  font-size: 12px;
  line-height: 1.45;
}

.window-controls .win-btn:last-child:hover {
  color: #fff;
  background: var(--app-color-china-red);
}
</style>
