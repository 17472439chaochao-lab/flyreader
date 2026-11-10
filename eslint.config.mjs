// ESLint 最小 flat 配置：只保证新增/改动文件通过，不重排全仓库格式
import js from '@eslint/js'
import tseslint from 'typescript-eslint'
import reactHooks from 'eslint-plugin-react-hooks'

export default tseslint.config(
  {
    ignores: ['node_modules/**', 'frontend/dist/**', 'src-tauri/**', 'target/**']
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ['frontend/src/**/*.{ts,tsx}', 'vitest.config.ts'],
    plugins: {
      'react-hooks': reactHooks
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' }
      ],
      '@typescript-eslint/no-explicit-any': 'warn'
    }
  }
)
