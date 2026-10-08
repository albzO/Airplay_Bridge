// 前端与桌面命令共享的数据约定。
export type Device = {
  name: string;
  service: string;
  addresses: string[];
  port: number;
  properties: Record<string, string>;
};
export type AudioFormat = {
  rate: number;
  channels: number;
  bits: number;
  valid_bits: number;
  encoding: string;
};
export type Input = {
  flow: 'recording' | 'playback';
  device_format: AudioFormat | null;
  mix_format: AudioFormat | null;
  id: string;
  name: string;
  channels: number | null;
  rate: number | null;
  encoding: string | null;
};
export type Settings = {
  endpoint: string;
  latency: number;
  buffer: number;
  mapping: [number, number];
  detailedLogs: boolean;
  captureDiagnostics: boolean;
  closeAction: 'tray' | 'quit';
  speakersSwapped: boolean;
  keepAwake: boolean;
};
