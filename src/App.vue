<script setup lang="ts">
import { onMounted, ref } from 'vue'

type Theme = 'dark' | 'light'

const theme = ref<Theme>('dark')

function applyTheme(next: Theme): void {
  theme.value = next
  document.documentElement.dataset['theme'] = next
}

function toggleTheme(): void {
  applyTheme(theme.value === 'dark' ? 'light' : 'dark')
}

onMounted(() => {
  applyTheme(theme.value)
})
</script>

<template>
  <main class="shell">
    <header class="shell__bar">
      <h1 class="shell__title">闭关模拟器</h1>
      <button class="btn" type="button" @click="toggleTheme">
        切换为{{ theme === 'dark' ? '暖白' : '暗色' }}
      </button>
    </header>

    <section class="shell__body">
      <p class="shell__lead">骨架阶段：先验证配色、字体与基础布局。</p>
      <p class="shell__sub">配置页与闭关页在 S7 接入，届时这里换成状态驱动的视图。</p>

      <div class="swatches">
        <div class="swatch swatch--bg"><span>底色</span><code>--bg</code></div>
        <div class="swatch swatch--fg"><span>主文字</span><code>--fg</code></div>
        <div class="swatch swatch--accent"><span>阵法金</span><code>--accent</code></div>
        <div class="swatch swatch--dialog"><span>对话框</span><code>--dialog-bg</code></div>
      </div>

      <p class="shell__clock num">系统 21:03 · 一致</p>
    </section>
  </main>
</template>

<style scoped>
.shell {
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: 24px 32px;
}

.shell__bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.shell__title {
  font-size: 18px;
  font-weight: 500;
  letter-spacing: 0.08em;
}

.shell__body {
  display: flex;
  flex: 1;
  flex-direction: column;
  justify-content: center;
  gap: 12px;
}

.shell__lead {
  font-size: 22px;
}

.shell__sub {
  color: var(--fg-muted);
  font-size: 13px;
}

.shell__clock {
  color: var(--fg-muted);
  font-size: 11px;
}

.swatches {
  display: flex;
  gap: 12px;
  margin-top: 16px;
}

.swatch {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 6px;
  padding: 16px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-card);
  font-size: 12px;
}

.swatch code {
  color: var(--fg-muted);
  font-family: var(--font-num);
  font-size: 11px;
}

.swatch--bg {
  background: var(--bg);
}

.swatch--fg {
  background: var(--fg);
  color: var(--bg);
}

.swatch--accent {
  background: var(--accent);
  color: #14110e;
}

.swatch--dialog {
  background: var(--dialog-bg);
}
</style>
