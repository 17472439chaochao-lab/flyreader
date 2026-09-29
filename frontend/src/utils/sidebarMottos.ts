/** 侧栏左下角轮换展示的阅读向短句 */
export const SIDEBAR_MOTTOS = [
  '读万卷书，行万里路。',
  '腹有诗书气自华。',
  '书犹药也，善读之可以医愚。',
  '黑发不知勤学早，白首方悔读书迟。',
  '三更灯火五更鸡，正是男儿读书时。',
  '旧书不厌百回读，熟读深思子自知。',
  '纸上得来终觉浅，绝知此事要躬行。',
  '问渠那得清如许，为有源头活水来。',
  '读书破万卷，下笔如有神。',
  '少壮不努力，老大徒伤悲。',
  '一日不读书，胸臆无佳想。',
  '开卷有益，静心自得。',
  '把时间留给文字，把安静留给自己。',
  '每一页翻过，都是多懂一点人间。',
  '今夜灯下，且读一章。'
] as const

/**
 * 按时间戳选取一句侧栏短句（同一时段内保持稳定）。
 * @param now 当前时间毫秒
 * @param slotMs 轮换间隔，默认 5 分钟
 */
export function pickSidebarMotto(now = Date.now(), slotMs = 5 * 60 * 1000): string {
  const i = Math.floor(now / slotMs) % SIDEBAR_MOTTOS.length
  return SIDEBAR_MOTTOS[i]
}
