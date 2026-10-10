<script setup lang="ts">
import { computed } from 'vue';
import type { Settings } from './types';

// 控件通过更新事件交回新值，不修改传入设置；保存、失败回退和系统副作用由 App 编排。
// Emit new values without mutating settings props; App owns persistence, rollback and system effects.
const props = defineProps<{
  settings: Settings;
  autostart: boolean;
  startupSaving: boolean;
  theme: string;
  channelCount: number;
  busy: boolean;
}>();
const emit = defineEmits<{
  'update:settings': [settings: Settings];
  'update:autostart': [enabled: boolean];
  'update:theme': [theme: string];
  'startup-change': [];
  'awake-change': [];
  persist: [];
  'mapping-change': [];
}>();
const autostart = computed({
  get: () => props.autostart,
  set: (value: boolean) => emit('update:autostart', value),
});
const theme = computed({
  get: () => props.theme,
  set: (value: string) => emit('update:theme', value),
});
const keepAwake = setting('keepAwake');
const closeAction = setting('closeAction');
const latency = setting('latency');
const leftChannel = mappedChannel(0);
const rightChannel = mappedChannel(1);
const channels = computed(() => Array.from({ length: props.channelCount }, (_, i) => i));

function setting<K extends keyof Settings>(key: K) {
  return computed({
    get: () => props.settings[key],
    set: (value: Settings[K]) => emit('update:settings', { ...props.settings, [key]: value }),
  });
}

function mappedChannel(side: 0 | 1) {
  return computed({
    get: () => props.settings.mapping[side],
    set: (value: number) => {
      const mapping: [number, number] = [...props.settings.mapping];
      mapping[side] = value;
      emit('update:settings', { ...props.settings, mapping });
    },
  });
}
</script>

<template>
  <section class="panel general-panel">
    <h2>应用行为</h2>
    <dl class="processing">
      <dt>开机自启</dt>
      <dd>
        <label class="log-switch"
          ><input
            type="checkbox"
            v-model="autostart"
            :disabled="startupSaving"
            @change="emit('startup-change')"
          />登录 Windows 后启动 AirPlay Hub</label
        ><small>默认关闭。启用后请保留软件所在位置。</small>
      </dd>
      <dt>保持系统唤醒</dt>
      <dd>
        <label class="log-switch"
          ><input
            type="checkbox"
            v-model="keepAwake"
            @change="emit('awake-change')"
          />避免系统自动睡眠</label
        ><small>开启后在应用运行期间生效，包括托盘状态；允许屏幕熄灭，退出应用后恢复。</small>
      </dd>
      <dt>右上角关闭按钮</dt>
      <dd>
        <select aria-label="窗口关闭动作" v-model="closeAction" @change="emit('persist')">
          <option value="tray">收起到托盘</option>
          <option value="quit">退出应用</option></select
        ><small>立即生效。托盘右键“退出”始终关闭应用。</small>
      </dd>
      <dt>外观</dt>
      <dd>
        <select aria-label="外观" v-model="theme">
          <option value="system">跟随系统</option>
          <option value="light">浅色模式</option>
          <option value="dark">深色模式</option>
        </select>
      </dd>
    </dl>
  </section>
  <section class="panel general-panel">
    <h2>播放设置</h2>
    <dl class="processing">
      <dt>播放提前量<small>下次连接生效</small></dt>
      <dd>
        <div class="unit small-unit">
          <input
            aria-label="播放提前量"
            v-model.number="latency"
            type="number"
            min="250"
            max="2000"
            :disabled="busy"
            @change="emit('persist')"
          /><span>ms</span>
        </div>
      </dd>
    </dl>
  </section>
  <section class="panel general-panel">
    <h2>输入声道映射</h2>
    <small>选择音频来源中的左右输入。交换扬声器位置不会改变这里的设置。</small>
    <dl class="processing">
      <dt>左输出取样</dt>
      <dd>
        <select
          aria-label="左输出取样"
          v-model.number="leftChannel"
          @change="emit('mapping-change')"
        >
          <option v-for="ch in channels" :value="ch">输入声道 {{ ch + 1 }}</option>
        </select>
      </dd>
      <dt>右输出取样</dt>
      <dd>
        <select
          aria-label="右输出取样"
          v-model.number="rightChannel"
          @change="emit('mapping-change')"
        >
          <option v-for="ch in channels" :value="ch">输入声道 {{ ch + 1 }}</option>
        </select>
      </dd>
    </dl>
  </section>
</template>
