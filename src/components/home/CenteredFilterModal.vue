<script setup lang="ts">
const props = withDefaults(defineProps<{
  visible: boolean
  width?: string
  modal?: boolean
  blurBackdrop?: boolean
  closeOnBackdrop?: boolean
  panelClass?: string
}>(), {
  modal: true,
  blurBackdrop: true,
  closeOnBackdrop: true,
})

const emit = defineEmits<{
  'update:visible': [value: boolean]
}>()

function close() {
  if (!props.closeOnBackdrop) return
  emit('update:visible', false)
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="props.visible"
      class="centered-filter-modal"
      :class="{ 'centered-filter-modal--non-modal': !props.modal }"
      @click="close"
    >
      <div
        v-if="props.modal"
        class="centered-filter-modal__backdrop"
        :class="{ 'centered-filter-modal__backdrop--flat': !props.blurBackdrop }"
        aria-hidden="true"
      ></div>
      <div
        class="centered-filter-modal__panel"
        :class="props.panelClass"
        :style="{ width: props.width || 'min(820px, calc(100vw - 32px))' }"
        @click.stop
      >
        <slot />
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.centered-filter-modal {
  position: fixed;
  inset: 0;
  z-index: 3000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
}

.centered-filter-modal--non-modal {
  pointer-events: none;
}

.centered-filter-modal__backdrop {
  position: absolute;
  inset: 0;
  background:
    radial-gradient(circle at 50% 38%, color-mix(in srgb, var(--app-accent-secondary) 18%, transparent), transparent 38%),
    color-mix(in srgb, var(--app-accent-deep) 36%, transparent);
  backdrop-filter: blur(14px) saturate(1.08);
}

.centered-filter-modal__backdrop--flat {
  background: color-mix(in srgb, var(--app-accent-deep) 9%, transparent);
  backdrop-filter: none;
}

.centered-filter-modal__panel {
  position: relative;
  z-index: 1;
  pointer-events: auto;
  max-width: calc(100vw - 32px);
  max-height: calc(100vh - 32px);
  overflow: auto;
  padding: 18px;
  border: 1px solid color-mix(in srgb, var(--app-border) 82%, white);
  border-radius: 22px;
  background:
    linear-gradient(180deg, color-mix(in srgb, var(--app-surface) 96%, white), var(--app-surface-soft));
  box-shadow: var(--app-shadow-lg);
  animation: filter-panel-in 0.16s ease-out;
}

@keyframes filter-panel-in {
  from {
    opacity: 0;
    transform: translateY(8px) scale(0.985);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}

@media (max-width: 900px) {
  .centered-filter-modal {
    padding: 10px;
  }

  .centered-filter-modal__panel {
    max-width: calc(100vw - 20px);
    max-height: calc(100vh - 20px);
    padding: 14px;
    border-radius: 14px;
  }
}
</style>
