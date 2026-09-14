<script setup lang="ts">
/**
 * DumpPanel ->Asset TypeTree Structured Data Display Panel
 *
 * When an asset is selected and the Dump tab is active, displays the TypeTree dump text for that asset.
 * Data is lazy-loaded via useWorkspace().fetchDumpData().
 */

import { Refresh } from '@element-plus/icons-vue'
import { useI18n } from 'vue-i18n'
import { useWorkspace } from '../../composables/home/useWorkspace'

const ws = useWorkspace()
const { t } = useI18n()
</script>

<template>
  <!-- No asset selected -->
  <div v-if="!ws.dumpTargetAsset.value" class="panel-empty dump-message">
    {{ t('dumpPanel.selectAsset') }}
  </div>

  <!-- Loading -->
  <div v-else-if="ws.dumpLoading.value" class="dump-message">
    <el-icon class="is-loading" :size="24"><Refresh /></el-icon>
    <p>{{ t('dumpPanel.extracting') }}</p>
  </div>

  <!-- Error state -->
  <div v-else-if="ws.dumpError.value" class="dump-message">
    <p class="dump-error-title">{{ t('dumpPanel.extractionFailed') }}</p>
    <pre class="dump-error-detail">{{ ws.dumpError.value }}</pre>
  </div>

  <!-- Dump content -->
  <div v-else-if="ws.dumpData.value" class="dump-container">
    <div class="dump-toolbar">
      <span class="dump-badge type-badge">
        {{ ws.dumpTargetAsset.value.class_name }}
      </span>
      <span v-if="ws.dumpData.value.truncated" class="dump-badge truncated-badge">
        {{ t('dumpPanel.truncated', { count: ws.dumpData.value.truncated_items }) }}
      </span>
      <span v-if="!ws.dumpData.value.has_typetree" class="dump-badge hex-badge">
        {{ t('dumpPanel.noTypeTree') }}
      </span>
      <span v-else class="dump-badge typetree-badge">
        {{ t('dumpPanel.typeTree') }}
      </span>
    </div>
    <el-scrollbar class="dump-scroll">
      <pre class="dump-text">{{ ws.dumpData.value.dump_text }}</pre>
    </el-scrollbar>
  </div>

  <!-- Empty state (asset selected but dumpData is null, before first switch) -->
  <div v-else class="panel-empty dump-message">
    {{ t('dumpPanel.clickDump') }}
  </div>
</template>

<style scoped>
/*
 * Dump Panel Styles
 */

/* Generic messages (empty/loading/error) */
.dump-message {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  flex: 1;
  gap: 8px;
  color: var(--el-text-color-secondary);
  padding: 24px;
}

.dump-error-title {
  font-weight: 600;
  color: var(--el-color-danger);
}

.dump-error-detail {
  font-family: 'Consolas', 'Courier New', monospace;
  font-size: 11px;
  color: var(--el-color-danger);
  max-width: 100%;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-all;
  background: var(--el-fill-color-lighter);
  padding: 8px 12px;
  border-radius: 4px;
  margin: 0;
}

/* Dump content container */
.dump-container {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

/* Top toolbar (badge row) */
.dump-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border-bottom: 1px solid var(--el-border-color-lighter);
  background: var(--el-fill-color-lighter);
  flex-shrink: 0;
  flex-wrap: wrap;
}

.dump-badge {
  font-size: 11px;
  font-weight: 600;
  padding: 1px 8px;
  border-radius: 4px;
  line-height: 1.6;
}

.type-badge {
  color: var(--el-color-primary);
  background: var(--el-color-primary-light-9);
}

.truncated-badge {
  color: var(--el-color-warning);
  background: var(--el-color-warning-light-9);
}

.hex-badge {
  color: var(--el-text-color-secondary);
  background: var(--el-fill-color-darker);
  font-style: italic;
}

.typetree-badge {
  color: var(--el-color-success);
  background: var(--el-color-success-light-9);
}

/* Text scroll area */
.dump-scroll {
  flex: 1;
  height: 100%;
}

.dump-text {
  font-family: 'Consolas', 'Courier New', 'Source Code Pro', monospace;
  font-size: 12px;
  line-height: 1.55;
  color: var(--el-text-color-primary);
  padding: 12px 16px;
  margin: 0;
  white-space: pre-wrap;
  word-break: break-all;
  tab-size: 4;
}
</style>
