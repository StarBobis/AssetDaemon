<script setup lang="ts">
import { Download, FolderOpened, Lock } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { DecryptPageViewUtils } from './DecryptPageViewUtils'
import { DecryptGameName } from '../types'

defineProps<{
  selectedGame: DecryptGameName
  inputPath: string
  outputPath: string
  concurrency: number
  isDecrypting: boolean
  gameOptions: DecryptGameName[]
}>()

defineEmits<{
  (event: 'update:selectedGame', value: DecryptGameName): void
  (event: 'update:inputPath', value: string): void
  (event: 'update:outputPath', value: string): void
  (event: 'update:concurrency', value: number): void
  (event: 'select-input-folder'): void
  (event: 'select-output-folder'): void
  (event: 'save-selected-game', value: DecryptGameName): void
  (event: 'save-concurrency', value: number): void
  (event: 'start-decryption'): void
}>()

const { t } = useI18n()
</script>

<template>
  <div class="decrypt-page">
    <div class="page-header">
      <el-icon class="header-icon" :size="28">
        <Lock />
      </el-icon>
      <div class="header-text">
        <h1>{{ t('decrypt.title') }}</h1>
        <p class="header-subtitle">
          {{ t('decrypt.subtitle') }}
        </p>
      </div>
    </div>

    <el-card class="config-card" shadow="never">
      <div class="config-row">
        <label class="config-label">
          {{ t('decrypt.game') }}
        </label>
        <el-select
          :model-value="selectedGame"
          class="game-select"
          :disabled="isDecrypting"
          @update:model-value="$emit('update:selectedGame', $event as DecryptGameName)"
          @change="$emit('save-selected-game', $event as DecryptGameName)"
        >
          <el-option
            v-for="game in gameOptions"
            :key="game"
            :label="game"
            :value="game"
          />
        </el-select>
      </div>

      <div class="config-row">
        <label class="config-label">
          <el-icon>
            <FolderOpened />
          </el-icon>
          {{ t('decrypt.inputDirectory') }}
        </label>
        <div class="picker-row">
          <el-input
            :model-value="inputPath"
            :placeholder="t('decrypt.inputPlaceholder')"
            clearable
            class="folder-input"
            :disabled="isDecrypting"
            @update:model-value="$emit('update:inputPath', DecryptPageViewUtils.toTextInputValue($event))"
          />
          <el-button
            type="primary"
            :disabled="isDecrypting"
            @click="$emit('select-input-folder')"
          >
            <el-icon class="btn-icon">
              <FolderOpened />
            </el-icon>
            {{ t('decrypt.selectFolder') }}
          </el-button>
        </div>
      </div>

      <div class="config-row">
        <label class="config-label">
          <el-icon>
            <Download />
          </el-icon>
          {{ t('decrypt.outputDirectory') }}
        </label>
        <div class="picker-row">
          <el-input
            :model-value="outputPath"
            :placeholder="t('decrypt.outputPlaceholder')"
            clearable
            class="folder-input"
            :disabled="isDecrypting"
            @update:model-value="$emit('update:outputPath', DecryptPageViewUtils.toTextInputValue($event))"
          />
          <el-button
            type="primary"
            :disabled="isDecrypting"
            @click="$emit('select-output-folder')"
          >
            <el-icon class="btn-icon">
              <FolderOpened />
            </el-icon>
            {{ t('decrypt.selectFolder') }}
          </el-button>
        </div>
      </div>

      <div class="config-row">
        <label class="config-label">
          {{ t('decrypt.maximumConcurrency') }}
          <el-tag size="small" type="info" class="concurrency-value">
            {{ concurrency }}
          </el-tag>
        </label>
        <div class="slider-row">
          <el-slider
            :model-value="concurrency"
            :min="1"
            :max="50"
            :step="1"
            :disabled="isDecrypting"
            show-stops
            :marks="{
              1: '1',
              10: '10',
              20: '20',
              30: '30',
              40: '40',
              50: '50',
            }"
            @update:model-value="$emit('update:concurrency', DecryptPageViewUtils.toSliderNumberValue($event))"
            @change="$emit('save-concurrency', DecryptPageViewUtils.toSliderNumberValue($event))"
          />
        </div>
      </div>
    </el-card>

    <div class="action-area">
      <el-button
        type="danger"
        size="large"
        :loading="isDecrypting"
        :disabled="isDecrypting"
        class="decrypt-button"
        @click="$emit('start-decryption')"
      >
        <el-icon class="btn-icon">
          <Lock />
        </el-icon>
        {{ isDecrypting ? t('decrypt.decrypting') : t('decrypt.decryptFiles') }}
      </el-button>
    </div>
  </div>
</template>

<style scoped>
.decrypt-page {
  max-width: 800px;
  margin: 0 auto;
  padding: 24px 20px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.page-header {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  margin-bottom: 4px;
}

.header-icon {
  margin-top: 4px;
  color: var(--el-color-primary);
}

.header-text h1 {
  margin: 0;
  font-size: 22px;
  font-weight: 600;
  line-height: 1.3;
}

.header-subtitle {
  margin: 4px 0 0;
  font-size: 13px;
  color: var(--el-text-color-secondary);
  line-height: 1.5;
}

.config-card {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 8px;
}

.config-row {
  margin-bottom: 16px;
}

.config-row:last-child {
  margin-bottom: 0;
}

.config-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 500;
  color: var(--el-text-color-primary);
  margin-bottom: 8px;
}

.concurrency-value {
  margin-left: 4px;
  font-weight: 600;
}

.picker-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.folder-input {
  flex: 1;
}

.game-select {
  width: 100%;
}

.slider-row {
  padding: 0 8px;
}

.btn-icon {
  margin-right: 4px;
}

.action-area {
  display: flex;
  justify-content: center;
}

.decrypt-button {
  min-width: 240px;
  font-size: 15px;
  font-weight: 600;
}
</style>
