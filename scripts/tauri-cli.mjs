import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const tauriDir = join(root, 'src-tauri')
const conf = join(tauriDir, 'tauri.conf.json')
const cli = join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js')

if (!existsSync(conf)) {
  console.error(
    `未找到 Tauri 配置：${conf}\n` +
      '请在仓库根目录执行 npm 脚本，并确认已完整 clone（必须包含 src-tauri/tauri.conf.json）。'
  )
  process.exit(1)
}

if (!existsSync(cli)) {
  console.error(`未找到本地 @tauri-apps/cli：${cli}\n请先在仓库根目录执行 npm install。`)
  process.exit(1)
}

const env = {
  ...process.env,
  TAURI_APP_PATH: tauriDir,
  TAURI_FRONTEND_PATH: root
}

const child = spawn(process.execPath, [cli, ...process.argv.slice(2)], {
  cwd: root,
  env,
  stdio: 'inherit',
  windowsHide: false
})

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal)
    return
  }
  process.exit(code ?? 1)
})
