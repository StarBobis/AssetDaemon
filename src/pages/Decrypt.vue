<script setup lang="ts">
import { onMounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import DecryptPageView from './DecryptPageView.vue'
import { DecryptPageController } from './DecryptPageController'
import { useLogSystem } from '../composables/useLogSystem'

const { t } = useI18n()
const { addLog, stopSignal } = useLogSystem()
const decrypt_page_controller = new DecryptPageController(t, addLog)

onMounted(decrypt_page_controller.loadSavedSettings)
watch(stopSignal, decrypt_page_controller.handleGlobalStopSignal)
</script>

<template>
  <DecryptPageView
    v-model:selected-game="decrypt_page_controller.selected_game.value"
    v-model:concurrency="decrypt_page_controller.concurrency.value"
    :input-path="decrypt_page_controller.input_path.value"
    :output-path="decrypt_page_controller.output_path.value"
    :is-decrypting="decrypt_page_controller.is_decrypting.value"
    :game-options="decrypt_page_controller.game_options"
    @update:input-path="decrypt_page_controller.handleInputPathChanged"
    @update:output-path="decrypt_page_controller.handleOutputPathChanged"
    @select-input-folder="decrypt_page_controller.selectInputFolder"
    @select-output-folder="decrypt_page_controller.selectOutputFolder"
    @save-concurrency="decrypt_page_controller.handleSaveConcurrency"
    @save-selected-game="decrypt_page_controller.handleSaveSelectedGame"
    @start-decryption="decrypt_page_controller.startDecryption"
  />
</template>
