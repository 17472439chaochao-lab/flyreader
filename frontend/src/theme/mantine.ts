import { createTheme, defaultCssVariablesResolver } from '@mantine/core'
import type {
  CSSVariablesResolver,
  MantineColorSchemeManager,
  MantineColorsTuple
} from '@mantine/core'

/**
 * Mantine 配色桥接：styles.css 的 --* 令牌仍是唯一色值来源（paper/green/night 由 html 的
 * class 切换），这里只把它们接进 Mantine 的 CSS 变量体系，不再抄一份色值。
 *
 * Mantine 取 shade 6（浅色方案）/ 8（深色方案）作实心色、7/9 作 hover，所以只在 7/9 上
 * 写 color-mix；混合目标用 --ink，夜间主题下 --ink 是亮色，hover 自然变成提亮。
 */
function tokenScale(token: string): MantineColorsTuple {
  const base = `var(--${token})`
  const hover = `color-mix(in srgb, ${base}, var(--ink) 16%)`
  return [base, base, base, base, base, base, base, hover, base, hover]
}

export const mantineTheme = createTheme({
  primaryColor: 'accent',
  defaultRadius: 'md',
  // 字体继承 app 在 :root 上声明的字体栈，避免两处维护
  fontFamily: 'inherit',
  // 标题走 --font-serif（styles.css 的 :root），与手写标题同一来源
  headings: { fontFamily: 'var(--font-serif)' },
  radius: { xs: '6px', sm: '8px', md: '10px', lg: '14px', xl: '18px' },
  colors: {
    accent: tokenScale('accent'),
    danger: tokenScale('danger'),
    ink: tokenScale('ink'),
    muted: tokenScale('ink-muted')
  },
  // 形状与字重统一在桥接层收敛：控件用 8px 小圆角、按钮 500 字重，
  // 手写 .btn 也改成同一圆角，避免迁移期两种形状混在一起。
  components: {
    Button: {
      defaultProps: { radius: 'sm' },
      styles: { root: { fontWeight: 500 } }
    },
    Select: { defaultProps: { radius: 'sm' } },
    // Modal 标题是 h2 + Mantine 自带的 16px/400 body 字体，跟手写弹窗标题
    // （.modal-head h3：衬线 20px 粗体）差一档，这里在桥接层一次性对齐。
    Modal: {
      styles: {
        title: { fontFamily: 'var(--font-serif)', fontSize: '20px', fontWeight: 700 }
      }
    },
    // 排行筛选用的 tab：pills 变体的选中态默认是实心 accent + 白字，一行里同时
    // 有四个选中项会太抢眼，这里压成 accent-soft 底 + accent 文字，未选中退回纯文字。
    // 变量必须挂在 list 上：--tabs-color 由 Tabs 的 varsResolver 写在 root 的行内样式里，
    // 主题 styles.root 压不住，而 tab 继承时以最近的祖先 list 为准。
    Tabs: {
      defaultProps: { variant: 'pills', radius: 'sm' },
      styles: {
        list: {
          '--tabs-color': 'var(--accent-soft)',
          '--tabs-text-color': 'var(--accent)',
          '--tab-color': 'var(--ink-muted)',
          '--tab-hover-color': 'color-mix(in srgb, var(--ink) 6%, transparent)'
        },
        tab: { padding: '6px 12px' }
      }
    },
    // Input 的 default 变体把底色/描边硬编码成 white / gray-4，比 app 的字段更亮更重；
    // TextInput、Select、Textarea 共用 Input，改这一处就全对齐了。
    Input: {
      styles: () => ({
        wrapper: {
          '--input-bd': 'var(--line)',
          '--input-bg': 'var(--surface-strong)',
          '--input-bd-focus': 'var(--accent)'
        }
      })
    }
  }
})

/** 语义变量：Mantine 的 light/dark 两块都会写同名变量，因此三块一并覆盖才压得住 */
const semanticVariables = {
  '--mantine-color-text': 'var(--ink)',
  '--mantine-color-dimmed': 'var(--ink-muted)',
  '--mantine-color-body': 'var(--bg-elevated)',
  '--mantine-color-anchor': 'var(--accent)',
  '--mantine-color-error': 'var(--danger)',
  '--mantine-color-placeholder': 'var(--ink-muted)',
  // default 变体（按钮、下拉等）用半透明白底 + 更淡的描边，对齐手写 .btn.ghost 的观感
  '--mantine-color-default': 'var(--surface)',
  '--mantine-color-default-hover': 'color-mix(in srgb, var(--ink) 6%, var(--surface))',
  '--mantine-color-default-color': 'var(--ink)',
  '--mantine-color-default-border': 'color-mix(in srgb, var(--ink) 7%, transparent)',
  '--mantine-color-disabled': 'var(--surface)',
  '--mantine-color-disabled-color': 'var(--ink-muted)',
  '--mantine-color-disabled-border': 'var(--line)'
}

/**
 * 中性色桥接：Mantine 的控件底色 / 描边 / 次级文字取自 gray-*（浅色方案）与 dark-*（深色方案），
 * 默认是冷灰，落在 paper / green / night 三套暖调主题上会显得脏。这两组变量只在 variables 块里
 * 生成，而 app 的令牌本身就随主题 class 切换，所以一处覆盖即可让两个方案都对齐。
 */
const neutralVariables = {
  '--mantine-color-gray-0': 'var(--bg-elevated)',
  '--mantine-color-gray-1': 'var(--surface-strong)',
  '--mantine-color-gray-2': 'color-mix(in srgb, var(--ink) 8%, var(--surface-strong))',
  '--mantine-color-gray-3': 'color-mix(in srgb, var(--ink) 12%, transparent)',
  '--mantine-color-gray-4': 'color-mix(in srgb, var(--ink) 24%, transparent)',
  '--mantine-color-gray-5': 'var(--ink-muted)',
  '--mantine-color-gray-6': 'var(--ink-muted)',
  '--mantine-color-gray-7': 'var(--ink)',
  '--mantine-color-gray-8': 'var(--ink)',
  '--mantine-color-gray-9': 'var(--ink)',
  '--mantine-color-dark-0': 'var(--ink)',
  '--mantine-color-dark-1': 'var(--ink)',
  '--mantine-color-dark-2': 'var(--ink-muted)',
  '--mantine-color-dark-3': 'var(--ink-muted)',
  // dark-4 在深色方案里充当描边与进度条底槽（不是文字色），用实色的 ink-muted 会过亮
  '--mantine-color-dark-4': 'color-mix(in srgb, var(--ink) 26%, transparent)',
  '--mantine-color-dark-5': 'var(--surface-strong)',
  '--mantine-color-dark-6': 'var(--bg-elevated)',
  '--mantine-color-dark-7': 'var(--bg-elevated)',
  '--mantine-color-dark-8': 'var(--bg)',
  '--mantine-color-dark-9': 'var(--bg)'
}

export const mantineCssVariables: CSSVariablesResolver = (theme) => {
  const base = defaultCssVariablesResolver(theme)
  return {
    variables: { ...base.variables, ...neutralVariables, ...semanticVariables },
    light: { ...base.light, ...semanticVariables },
    dark: { ...base.dark, ...semanticVariables }
  }
}

/**
 * 深浅色由 settings.theme 决定（App 的主题 effect 里 setColorScheme），
 * 用内存 manager 顶掉默认的 localStorage + 系统偏好监听，避免出现第二套主题来源。
 */
export const appColorSchemeManager: MantineColorSchemeManager = {
  get: (defaultValue) => defaultValue,
  set: () => {},
  subscribe: () => {},
  unsubscribe: () => {},
  clear: () => {}
}
