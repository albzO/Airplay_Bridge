<script setup lang="ts">
import { computed } from 'vue';
import { num } from './display';
import type { SessionReport, Telemetry } from './types';

// 只读展示当前遥测/报告和跨会话累计计数；App 校验事件归属，useDiagnostics 计算增量。
// Display current telemetry/reports and cumulative counters; App validates attribution; useDiagnostics computes deltas.
const props = defineProps<{
  telemetry: Partial<Telemetry>;
  report: SessionReport;
  stats: Record<string, Record<string, string>>;
  names: Record<string, string>;
}>();
const statsRows = computed(() =>
  Object.entries(props.stats).map(([host, values]) => ({
    host,
    values,
    name: props.names[host] || host,
  })),
);
</script>

<template>
  <div class="metrics">
    <section class="panel">
      <small>运行时长</small>
      <h2>{{ num(telemetry.elapsed_seconds, 0) }} <em>s</em></h2>
    </section>
    <section class="panel">
      <small>流水线水位</small>
      <h2>{{ num(telemetry.water_ms) }} <em>ms</em></h2>
    </section>
    <section class="panel">
      <small>漂移校正</small>
      <h2>{{ num(telemetry.controller?.correction_ppm) }} <em>ppm</em></h2>
    </section>
  </div>
  <section class="panel">
    <h2>音频流水线</h2>
    <dl>
      <dt>采集帧数</dt>
      <dd>{{ telemetry.capture_frames?.toLocaleString() || '—' }}</dd>
      <dt>输出帧数</dt>
      <dd>{{ telemetry.output_frames?.toLocaleString() || '—' }}</dd>
      <dt>协议提前量</dt>
      <dd>{{ telemetry.lead_ms ?? '—' }} ms</dd>
      <dt>估计发送端延迟</dt>
      <dd>{{ num((telemetry.lead_ms ?? NaN) + (telemetry.water_ms ?? NaN)) }} ms</dd>
      <dt>采集不连续 / 时间戳错误</dt>
      <dd>
        {{ report.capture?.discontinuities ?? '停止后汇总' }} /
        {{ report.capture?.timestamp_errors ?? '—' }}
      </dd>
      <dt>转换汇总</dt>
      <dd>
        <code>{{ report.conversion ? JSON.stringify(report.conversion) : '停止后汇总' }}</code>
      </dd>
    </dl>
    <small>延迟估计不包含应用和扬声器实际发声耗时。</small>
  </section>
  <section class="panel">
    <h2>设备传输</h2>
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>设备</th>
            <th>发送包</th>
            <th>本地丢包</th>
            <th>重传请求</th>
            <th>已重传</th>
            <th>过期</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in statsRows" :key="row.host">
            <td>{{ row.name }}</td>
            <td>{{ row.values.sent ?? '—' }}</td>
            <td>{{ row.values.send_dropped ?? '—' }}</td>
            <td>{{ row.values.rtx_requested ?? '—' }}</td>
            <td>{{ row.values.rtx_resent ?? '—' }}</td>
            <td>{{ row.values.rtx_expired ?? '—' }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <small>约每 5 秒更新，累计本次运行中最近 128 个设备的统计，退出软件后清空。</small>
  </section>
</template>
