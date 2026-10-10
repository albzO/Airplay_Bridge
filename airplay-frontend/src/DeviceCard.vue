<script setup lang="ts">
import type { DeviceCardData } from './types';

// 只负责设备展示与操作事件；选择、会话状态和桌面命令由 App 编排。
// Present devices and emit actions; App owns selection, session state and desktop commands.
const props = defineProps<{
  card: DeviceCardData;
  state?: string;
  selected: boolean;
  expanded: boolean;
  disabled: boolean;
  active: boolean;
  swapped: boolean;
  swapDisabled: boolean;
  mapping: [number, number];
  peaks: number[];
  levelLabels: [string, string];
}>();
const emit = defineEmits<{
  choose: [];
  swap: [];
}>();

function speakerFor(side: number) {
  const members = props.card.members;
  return members[members.length === 2 && props.swapped ? 1 - side : side] || members[0];
}
</script>

<template>
  <article :class="['device', state, { selected }]">
    <button class="device-row" :disabled="disabled" @click="emit('choose')">
      <span class="radio">{{ selected ? '●' : '○' }}</span
      ><span class="speaker"
        ><svg class="device-speaker-icon" viewBox="0 0 44 36" aria-hidden="true">
          <template v-if="card.members.length === 2">
            <circle cx="16" cy="14" r="11" mask="url(#stereo-icon-gap)" />
            <circle cx="28" cy="23" r="11" />
          </template>
          <circle v-else cx="22" cy="18" r="12" /></svg
      ></span>
      <div class="device-name">
        <strong>{{ card.title }}</strong
        ><small>{{ card.members.length === 2 ? '立体声对' : card.members[0]?.addresses[0] }}</small>
      </div>
      <span class="badge">{{ card.members.length === 2 ? 'Stereo' : 'AirPlay 2' }}</span
      ><span>{{ expanded ? '⌃' : '⌄' }}</span>
    </button>
    <div v-if="expanded" class="device-details">
      <div v-if="active" :class="['channel-pair', { single: card.members.length === 1 }]">
        <div>
          <small>左{{ card.members.length === 2 ? '扬声器' : '声道' }}</small
          ><strong>{{ speakerFor(0)?.name }}</strong>
          <div class="meter">
            <i :style="{ width: Math.min(100, peaks[0] * 100) + '%' }"></i>
          </div>
          <small>{{ levelLabels[0] }} · 输入 {{ mapping[0] + 1 }}</small>
        </div>
        <button
          v-if="card.members.length === 2"
          title="交换扬声器位置"
          :disabled="swapDisabled"
          @click="emit('swap')"
        >
          ⇄
        </button>
        <div>
          <small>右{{ card.members.length === 2 ? '扬声器' : '声道' }}</small
          ><strong>{{ speakerFor(1)?.name }}</strong>
          <div class="meter">
            <i :style="{ width: Math.min(100, peaks[1] * 100) + '%' }"></i>
          </div>
          <small>{{ levelLabels[1] }} · 输入 {{ mapping[1] + 1 }}</small>
        </div>
      </div>
      <div v-else class="device-info">
        {{ card.members.map((d) => `${d.name} · ${d.addresses[0]}:${d.port}`).join(' / ') }}
      </div>
    </div>
  </article>
</template>
