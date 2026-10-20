import js from '@eslint/js'
import pluginVue from 'eslint-plugin-vue'
import prettier from 'eslint-config-prettier'
import globals from 'globals'
import tseslint from 'typescript-eslint'

export default tseslint.config(
  {
    ignores: ['dist/**', 'node_modules/**', 'src-tauri/target/**', 'src-tauri/gen/**'],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...pluginVue.configs['flat/recommended'],
  {
    files: ['**/*.vue'],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser,
        extraFileExtensions: ['.vue'],
      },
    },
  },
  {
    languageOptions: {
      globals: { ...globals.browser },
    },
    rules: {
      // 单文件视图组件用 Setup/Lock 命名，不强制多词
      'vue/multi-word-component-names': 'off',
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }],
    },
  },
  {
    files: ['*.config.js', '*.config.ts', 'vite.config.ts'],
    languageOptions: {
      globals: { ...globals.node },
    },
  },
  // 必须放最后：关掉与 Prettier 冲突的格式类规则，格式只由 Prettier 判定
  prettier,
)
