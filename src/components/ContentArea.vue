<script setup lang="ts">
/**
 * ContentArea owns everything below the custom title bar.
 *
 * Keeping routed pages and shared bottom panels inside this component gives the
 * app a real titlebar row plus a separate content row, so page-level overlays
 * can be constrained to the content area instead of starting at viewport y=0.
 */

import LogPanel from './LogPanel.vue'
import { useGlobalLogLayout } from '../composables/useGlobalLogLayout'

const logLayout = useGlobalLogLayout()
</script>

<template>
  <section class="content-area">
    <main class="main-content">
      <router-view v-slot="{ Component }">
        <keep-alive>
          <component :is="Component" />
        </keep-alive>
      </router-view>
    </main>
    <div
      class="global-log-splitter"
      @mousedown.prevent="logLayout.startDragLogHeight"
    >
      <div class="global-log-splitter-handle"></div>
    </div>
    <div
      class="global-log-panel-wrap"
      :style="{ height: logLayout.logHeight.value + 'px' }"
    >
      <LogPanel />
    </div>
  </section>
</template>

<style scoped>
.content-area {
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.main-content {
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.global-log-splitter {
  height: 6px;
  flex-shrink: 0;
  cursor: row-resize;
  position: relative;
  border-top: 1px solid var(--app-border-soft);
  background: var(--app-surface-soft);
  transition: background 0.15s;
  z-index: 2;
}

.global-log-splitter:hover {
  background: var(--el-border-color-extra-light);
}

.global-log-splitter-handle {
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  width: 24px;
  height: 3px;
  border-radius: 2px;
  background: var(--el-text-color-placeholder);
  box-shadow:
    0 -5px 0 var(--el-text-color-placeholder),
    0 5px 0 var(--el-text-color-placeholder);
  opacity: 0;
  transition: opacity 0.15s;
  pointer-events: none;
}

.global-log-splitter:hover .global-log-splitter-handle,
.global-log-splitter:active .global-log-splitter-handle {
  opacity: 0.6;
}

.global-log-panel-wrap {
  flex-shrink: 0;
  min-height: 80px;
  max-height: 520px;
  display: flex;
  overflow: hidden;
  border-top: 1px solid var(--app-border-soft);
}
</style>
