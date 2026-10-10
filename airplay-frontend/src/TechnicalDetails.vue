<script setup lang="ts">
import { computed } from 'vue';
import { formatAudioFormat, level } from './display';
import type { Device, Input } from './types';

// 展示格式、电平与设备能力，Buffer 用受控更新；保存和认证策略命令由 App 编排。
// Present formats, levels and capabilities with a controlled Buffer; App owns persistence and auth policy commands.
const props = defineProps<{
  source?: Input;
  buffer: number;
  busy: boolean;
  saving: boolean;
  canResetAuth: boolean;
  authNotice: string;
  peaks: number[];
  technical: Record<string, string>;
  devices: Device[];
}>();
const emit = defineEmits<{
  'update:buffer': [value: number];
  persist: [];
  'reset-auth': [];
}>();
const buffer = computed({
  get: () => props.buffer,
  set: (value: number) => emit('update:buffer', value),
});
</script>

<template>
  <section class="panel capture-panel">
    <h2>采集与处理</h2>
    <dl class="processing">
      <dt>设备格式</dt>
      <dd>{{ formatAudioFormat(source?.device_format) }}</dd>
      <dt>WASAPI 采集格式</dt>
      <dd>{{ formatAudioFormat(source?.mix_format) }}</dd>
      <dt>发送格式</dt>
      <dd>44100 Hz · 16-bit PCM → ALAC</dd>
      <dt>Buffer<small>下次连接生效</small></dt>
      <dd>
        <div class="unit small-unit" title="采集流水线预缓冲">
          <input
            aria-label="采集 Buffer"
            v-model.number="buffer"
            type="number"
            min="64"
            max="512"
            :disabled="busy || saving"
            @change="emit('persist')"
          /><span>ms</span>
        </div>
      </dd>
      <dt>左右电平</dt>
      <dd>
        {{ level(peaks[0]) }} / {{ level(peaks[1]) }}
        <div class="meter">
          <i :style="{ width: Math.min(100, Math.max(...peaks) * 100) + '%' }"></i>
        </div>
      </dd>
      <dt>密码要求记录</dt>
      <dd>
        <button :disabled="busy || !canResetAuth" @click="emit('reset-auth')">
          重新检测所选设备</button
        ><small>{{ authNotice || '只记住密码要求，不保存密码。取消设备密码后可重新检测。' }}</small>
      </dd>
      <dt>会话协议</dt>
      <dd>{{ JSON.stringify(technical) }}</dd>
    </dl>
  </section>
  <section class="panel">
    <h2>设备能力</h2>
    <details v-for="d in devices">
      <summary>{{ d.name }} · {{ d.addresses.join(', ') }}:{{ d.port }}</summary>
      <pre>{{ JSON.stringify(d.properties, null, 2) }}</pre>
    </details>
  </section>
</template>
