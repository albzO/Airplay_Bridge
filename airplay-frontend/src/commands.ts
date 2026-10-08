import { invoke } from '@tauri-apps/api/core';
import { describeError } from './errors';

// 统一命令错误格式，页面只负责展示，不重复处理底层错误来源。
// Normalize command errors centrally so pages only display them without duplicating error mapping.
export async function invokeCommand<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(name, args);
  } catch (error) {
    throw new Error(describeError(error, name));
  }
}
