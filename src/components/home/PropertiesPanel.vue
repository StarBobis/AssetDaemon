<script setup lang="ts">
/**
 * PropertiesPanel ->Bottom-right properties panel
 *
 * Shows detailed information for the currently selected asset:
 * name, type, class ID, path ID, data size, etc.
 * Mesh assets additionally show geometry info like vertex count, triangle count.
 */

import { useWorkspace } from '../../composables/home/useWorkspace'
import { useI18n } from 'vue-i18n'

const ws = useWorkspace()
const { t } = useI18n()
</script>

<template>
  <!-- Properties panel header -->
  <div class="panel-header">{{ t('home.properties') }}</div>

  <!-- Property list or empty state -->
  <el-scrollbar class="properties-scroll">
    <div v-if="Object.keys(ws.assetProperties.value).length === 0" class="panel-empty">
      {{ t('home.noProperties') }}
    </div>
    <div v-else class="prop-list">
      <div
        v-for="(value, key) in ws.assetProperties.value"
        :key="key"
        class="prop-item"
      >
        <span class="prop-key">{{ key }}</span>
        <span class="prop-value">{{ value }}</span>
      </div>
    </div>
  </el-scrollbar>
</template>
