<script setup lang="ts">
import { computed } from 'vue';
import type { Input } from './types';

// 只负责来源展示与选择事件；设置、采集命令和弹窗关闭状态由 App 编排。
// Only source presentation and selection events. App owns settings, capture commands and dismissal state.
const props = defineProps<{
  inputs: Input[];
  endpoint: string;
  open: boolean;
  disabled: boolean;
  meterWidth: number;
}>();
const emit = defineEmits<{
  'update:open': [value: boolean];
  select: [input: Input];
}>();
const source = computed(() => props.inputs.find((input) => input.id === props.endpoint));
const recordingInputs = computed(() =>
  props.inputs.filter((i) => i.flow === 'recording').sort(sortSources),
);
const playbackInputs = computed(() =>
  props.inputs.filter((i) => i.flow === 'playback').sort(sortSources),
);
const sourceGroups = computed(() => [
  {
    flow: 'playback',
    title: '播放设备',
    english: 'Playback',
    description: '采集此设备正在播放的声音',
    items: playbackInputs.value,
  },
  {
    flow: 'recording',
    title: '录音设备',
    english: 'Recording',
    description: '采集此设备的输入声音',
    items: recordingInputs.value,
  },
]);

function sortSources(a: Input, b: Input) {
  return (
    a.name.localeCompare(b.name, 'en', { sensitivity: 'base', numeric: true }) ||
    a.id.localeCompare(b.id)
  );
}

function sourceIcon(flow: string) {
  return flow === 'playback'
    ? 'M11 4 6 8H3v8h3l5 4V4Zm4 4a6 6 0 0 1 0 8m3-11a10 10 0 0 1 0 14'
    : 'M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3ZM6 11v1a6 6 0 0 0 12 0v-1m-6 7v3m-3 0h6';
}
</script>

<template>
  <div class="source-picker" @keydown.esc="emit('update:open', false)">
    <div v-if="open" class="source-popup" id="source-options" aria-label="音频来源选项">
      <section
        v-for="group in sourceGroups"
        :key="group.flow"
        :class="['source-section', group.flow]"
        role="group"
        :aria-label="group.title"
      >
        <div class="source-group">
          <svg class="source-icon" viewBox="0 0 24 24" aria-hidden="true">
            <path :d="sourceIcon(group.flow)" />
          </svg>
          <div>
            <strong
              >{{ group.title }} <small>{{ group.english }}</small></strong
            ><small>{{ group.description }}</small>
          </div>
        </div>
        <button
          v-for="input in group.items"
          :key="input.id"
          :disabled="disabled || !input.channels"
          :aria-pressed="endpoint === input.id"
          @click="emit('select', input)"
        >
          <span>{{ input.name }}</span
          ><span class="source-check" aria-hidden="true">{{
            endpoint === input.id ? '✓' : ''
          }}</span></button
        ><small v-if="!group.items.length" class="source-empty">没有可用设备</small>
      </section>
    </div>
    <button
      class="source-trigger"
      :disabled="disabled"
      :aria-expanded="open"
      aria-controls="source-options"
      :aria-labelledby="
        source ? 'source-label source-kind source-name' : 'source-label source-name'
      "
      @click="emit('update:open', !open)"
    >
      <span class="source-selection"
        ><span v-if="source" id="source-kind" :class="['source-type', source.flow]"
          ><svg class="source-icon" viewBox="0 0 24 24" aria-hidden="true">
            <path :d="sourceIcon(source.flow)" /></svg
          >{{ source.flow === 'playback' ? '播放' : '录音' }}</span
        ><span id="source-name">{{ source?.name || '请选择音频流来源' }}</span></span
      ><svg viewBox="0 0 16 16">
        <path :d="open ? 'm4 6 4 4 4-4' : 'm4 10 4-4 4 4'" /></svg
      ><span class="source-total-meter"><i :style="{ width: meterWidth + '%' }"></i></span>
    </button>
  </div>
</template>
