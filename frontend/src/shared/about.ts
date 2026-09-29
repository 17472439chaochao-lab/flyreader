/** 应用身份信息与开源致谢（关于页展示用）。 */
export const APP_ABOUT = {
  name: 'FlyReader',
  version: '1.1.0',
  author: '飞鸟传说',
  qq: '17472439',
  email: '17472439chaochao@gmail.com',
  repo: 'https://gitee.com/wucc513721/flyreader',
  /** Gitee 仓库路径，用于 Releases 版本检查 */
  giteeOwner: 'wucc513721',
  giteeRepo: 'flyreader',
  /** 发行版列表页（无最新版 API 时的兜底打开地址） */
  releasesUrl: 'https://gitee.com/wucc513721/flyreader/releases',
  license: 'MIT',
  tagline: '轻量小说阅读器 · 支持 Legado 书源',
  features:
    '支持本地 TXT / EPUB / Markdown 文档阅读，在线小说搜索与阅读，以及导出功能。',
  opensourceNote:
    '本软件为开源项目，采用 MIT 许可证发布。欢迎学习、使用与二次开发；请在衍生作品中保留原作者信息与开源声明。',
  components: [
    { name: 'Tauri 2', desc: '跨平台桌面应用壳（系统 WebView）', license: 'MIT' },
    { name: 'React', desc: '界面框架', license: 'MIT' },
    { name: 'TypeScript', desc: '类型系统与编译', license: 'Apache-2.0' },
    { name: 'Vite', desc: '开发构建工具链', license: 'MIT' },
    { name: 'Rust + rusqlite', desc: '后端与本地 SQLite 数据存储', license: 'MIT' },
    { name: 'reqwest / scraper', desc: 'HTTP 与 HTML 解析（书源规则）', license: 'MIT / Apache-2.0' },
    { name: 'quickjs (rquickjs)', desc: '@js 书源规则沙箱执行', license: 'MIT' },
    { name: 'encoding_rs', desc: '网页编码转换', license: 'MIT / Apache-2.0' },
    { name: 'zip', desc: 'EPUB（ZIP）解压', license: 'MIT' },
    { name: 'marked', desc: 'Markdown 解析（GFM）', license: 'MIT' },
    { name: 'dompurify', desc: 'Markdown HTML 消毒', license: 'MPL-2.0 / Apache-2.0' }
  ]
} as const
