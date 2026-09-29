/**
 * 进度事件订阅工具：把一次异步操作的完整生命周期与主进程进度事件绑定，
 * 任务完成或抛错后自动取消订阅，避免各处重复「订阅 → try/finally 退订」样板。
 */

/** 与主进程进度事件载荷兼容的最小子集 */
export type ProgressPayload = {
  done: number
  total: number
  current: string
  phase: string
}

/** 统一的进度文案：done 显示「完成」，否则为 done/total · current */
export function formatProgressLabel(p: ProgressPayload): string {
  return p.phase === 'done' ? '完成' : `${p.done}/${p.total}${p.current ? ` · ${p.current}` : ''}`
}

/**
 * 执行带进度订阅的异步任务：订阅随任务开始，随任务结束（含异常）自动退订。
 * @param subscribe 用 track 登记若干事件订阅（track 接收各订阅返回的退订函数）
 * @param task 实际执行的异步操作
 * @returns task 的返回值
 */
export async function runWithProgress<T>(
  subscribe: (track: (off: () => void) => void) => void,
  task: () => Promise<T>
): Promise<T> {
  const offs: Array<() => void> = []
  const track = (off: () => void) => offs.push(off)
  subscribe(track)
  try {
    return await task()
  } finally {
    for (const off of offs) off()
  }
}
