import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const tsconfig = join(root, 'tsconfig.json')
const tscJs = join(root, 'node_modules', 'typescript', 'bin', 'tsc')
const viteJs = join(root, 'node_modules', 'vite', 'bin', 'vite.js')

function run(bin, args) {
  const result = spawnSync(process.execPath, [bin, ...args], {
    cwd: root,
    stdio: 'inherit',
    windowsHide: false
  })
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}

if (existsSync(tsconfig)) {
  if (!existsSync(tscJs)) {
    console.error(`未找到 typescript：${tscJs}\n请先在仓库根目录执行 npm install。`)
    process.exit(1)
  }
  run(tscJs, ['--noEmit', '-p', tsconfig])
} else {
  console.warn(
    `未找到 ${tsconfig}，已跳过 tsc 类型检查。` +
      '请在仓库根目录执行：git restore --source=origin/main -- tsconfig.json'
  )
}

if (!existsSync(viteJs)) {
  console.error(`未找到 vite：${viteJs}\n请先在仓库根目录执行 npm install。`)
  process.exit(1)
}

run(viteJs, ['build'])
