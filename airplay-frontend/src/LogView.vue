<script setup lang="ts">
import { computed } from 'vue';
import type { SessionReport } from './types';

// 只展示日志/报告并发出操作事件；App 管理设置保存，useDiagnostics 管理快照和清空。
// Present logs/reports and emit actions; App owns persistence; useDiagnostics owns snapshots and clearing.
const props = defineProps<{
  detailedLogs: boolean;
  captureDiagnostics: boolean;
  busy: boolean;
  activeDetailedLogs: boolean;
  activeDiagnostics: boolean;
  logs: string[];
  diagnosticLogs: string[];
  logPath: string;
  pipelinePath: string;
  diagnosticPath: string;
  report: SessionReport;
}>();
const emit = defineEmits<{
  'update:detailedLogs': [enabled: boolean];
  'update:captureDiagnostics': [enabled: boolean];
  persist: [];
  'open-logs': [];
  'clear-diagnostics': [];
}>();
const detailedLogs = computed({
  get: () => props.detailedLogs,
  set: (value: boolean) => emit('update:detailedLogs', value),
});
const captureDiagnostics = computed({
  get: () => props.captureDiagnostics,
  set: (value: boolean) => emit('update:captureDiagnostics', value),
});
</script>

<template>
  <section class="panel">
    <div class="section-title">
      <h2>会话日志</h2>
      <button @click="emit('open-logs')">打开日志目录</button>
    </div>
    <label class="log-switch"
      ><input
        type="checkbox"
        v-model="detailedLogs"
        :disabled="busy"
        @change="emit('persist')"
      />保存详细日志</label
    ><small
      >下次连接生效。开启后保存详细协议日志和每秒采集、重采样、队列、发送进度；关闭时只保存关键故障。</small
    ><small class="log-location">{{ logPath }}</small
    ><small v-if="pipelinePath" class="log-location">流水线：{{ pipelinePath }}</small
    ><small
      >显示最近 300 行{{
        activeDetailedLogs ? '详细输出。文件有大小与保留限制，旧日志会清理。' : '关键故障。'
      }}</small
    >
    <pre class="logs">{{
      logs.join('\n') || (activeDetailedLogs ? '等待详细日志输出…' : '暂无关键故障。')
    }}</pre>
    <details v-if="report.device">
      <summary>完整会话报告</summary>
      <pre>{{ JSON.stringify(report, null, 2) }}</pre>
    </details>
  </section>
  <section class="panel">
    <div class="section-title">
      <h2>采集逐包诊断</h2>
      <button @click="emit('clear-diagnostics')">清空显示</button>
    </div>
    <label class="log-switch"
      ><input
        type="checkbox"
        v-model="captureDiagnostics"
        :disabled="busy"
        @change="emit('persist')"
      />启用额外采集诊断</label
    ><small
      >下次连接生效，独立于“保存详细日志”。保存逐包时序、帧数、内容指纹、处理耗时和队列进度；不保存原始音频。滚动保存当前和前
      3 个片段，每片段约 8 MiB，故障时保留最近现场。</small
    ><small v-if="diagnosticPath" class="log-location">逐包：{{ diagnosticPath }}</small
    ><small>窗口每秒更新，保留最近 120 条摘要；逐包数据保存到文件。</small>
    <pre class="logs">{{
      diagnosticLogs.join('\n\n') || (activeDiagnostics ? '等待采集诊断输出…' : '未启用采集诊断。')
    }}</pre>
  </section>
</template>
