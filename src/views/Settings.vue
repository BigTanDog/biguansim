<script setup lang="ts">
// 设置页（PRD F-10）：五个分组，全部即时生效并落盘。
// 关于分组照 PRD §13 如实列出"拦不住什么"——这是产品承诺的一部分，不许美化。
import { computed, ref } from 'vue'

import * as ipc from '../lib/ipc'
import { useSessionStore } from '../stores/session'

const store = useSessionStore()

const config = computed(() => store.config)
const saving = ref(false)

/**
 * PRD §13 的九条局限。**措辞收敛、事实不删**（PRD 原要求"原文展示"，
 * 2026-10-21 你要求写得委婉些，免得把逃课门路教给不熟电脑的人，见 D-47）：
 * 每条都保留了"能做到什么程度"的边界，不写具体按键与操作步骤。
 */
const LIMITS = [
  '系统级的安全界面不归本程序管，从那里离开是 Windows 的设计，不是漏洞。',
  '触控板的多指手势，在部分机器上仍可切换窗口。',
  '安全模式、断电、长按电源键、另一个管理员账户——这些都能绕过它。',
  '闭关时显示器可能自动黑屏，那是系统节能，不是死机。',
  'v0.1 只管本机；手机、另一台电脑，它管不了。',
  '题库与记录都是明文：技术上你可以改答案、删掉统计来重置衰减。不加密、也不做完整性校验。',
  'Win 键那类系统级入口仍可能唤出开始菜单或桌面，程序只保证 1 秒内把窗口抢回来。请不要把它当作"能防住自己"的依据。',
  '它不是沙箱也不是家长控制：本质是"防手滑"，不是"防黑客"。',
  '任何"帮你专注"的工具，最终都只是把选择推到你面前一次。',
]

async function patch(next: Record<string, unknown>): Promise<void> {
  if (saving.value) {
    return
  }
  saving.value = true
  await store.saveConfig(next as Partial<ipc.Config>)
  saving.value = false
}

async function openDir(): Promise<void> {
  try {
    await ipc.openDataDir()
  } catch (err) {
    store.notify('error', String(err))
  }
}
</script>

<template>
  <main class="settings">
    <header class="settings__head">
      <button class="settings__back" type="button" @click="store.closeSubPage()">‹ 返回</button>
      <h1 class="settings__title">设置</h1>
    </header>

    <section v-if="config" class="card">
      <h2 class="card__title">显示</h2>
      <div class="row">
        <span class="row__label">主题</span>
        <div class="seg">
          <button
            class="seg__item"
            :class="{ 'seg__item--on': config.theme !== 'light' }"
            type="button"
            @click="patch({ theme: 'dark' })"
          >
            暗色
          </button>
          <button
            class="seg__item"
            :class="{ 'seg__item--on': config.theme === 'light' }"
            type="button"
            @click="patch({ theme: 'light' })"
          >
            暖白
          </button>
        </div>
      </div>
      <p class="row__note">
        入关时界面会再压暗一层（纯界面效果，不动系统亮度——台式机改不了，见关于）。
      </p>
    </section>

    <section v-if="config" class="card">
      <h2 class="card__title">退出成本</h2>
      <div class="row">
        <span class="row__label">冷却基数（分钟）</span>
        <input
          class="input num"
          type="number"
          min="3"
          max="10"
          :value="config.cooldownBase"
          @change="patch({ cooldownBase: Number(($event.target as HTMLInputElement).value) })"
        />
      </div>
      <div class="row">
        <span class="row__label">题数基数</span>
        <input
          class="input num"
          type="number"
          min="3"
          max="5"
          :value="config.questionBase"
          @change="patch({ questionBase: Number(($event.target as HTMLInputElement).value) })"
        />
      </div>
      <p class="row__note">
        第 n 次尝试的冷却是 基数 × n（封顶 20 分钟），题数是 基数 + (n−1)（封顶 5 题）。
      </p>
    </section>

    <section v-if="config" class="card">
      <h2 class="card__title">声音</h2>
      <div class="row">
        <span class="row__label">钟鸣音量</span>
        <input
          class="input"
          type="range"
          min="0"
          max="0.6"
          step="0.05"
          :value="config.chimeVolume"
          @change="patch({ chimeVolume: Number(($event.target as HTMLInputElement).value) })"
        />
      </div>
      <p class="row__note">上限 60%：到点三声钟鸣，末 10 秒三声轻滴答。系统静音时听不到。</p>
    </section>

    <section class="card">
      <h2 class="card__title">数据</h2>
      <div class="row">
        <span class="row__label">数据目录</span>
        <button class="link" type="button" @click="openDir">打开所在文件夹</button>
      </div>
      <p class="row__note">配置、记录、题库统计都在这里；记录是纯文本 JSONL，可自行备份或查看。</p>
    </section>

    <section class="card">
      <h2 class="card__title">关于</h2>
      <p class="about__line">闭关模拟器 v0.1 · 离线单机 · 不收集任何数据</p>
      <p class="about__sub">它拦不住什么（PRD §13）：</p>
      <ul class="limits">
        <li v-for="item in LIMITS" :key="item">{{ item }}</li>
      </ul>
    </section>
  </main>
</template>

<style scoped>
.settings {
  display: flex;
  flex-direction: column;
  gap: 12px;
  width: min(720px, 100%);
  height: 100%;
  margin: 0 auto;
  padding: 26px 24px;
  overflow-y: auto;
}

.settings__head {
  display: flex;
  align-items: center;
  gap: 12px;
}

.settings__back {
  padding: 4px 10px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: transparent;
  color: var(--fg-muted);
  font-size: 12px;
}

.settings__title {
  font-size: 18px;
  font-weight: 500;
  letter-spacing: 0.06em;
}

.card {
  padding: 14px 16px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-card);
  background: var(--dialog-bg);
}

.card__title {
  margin-bottom: 10px;
  color: var(--fg-muted);
  font-size: 11px;
  font-weight: 400;
  letter-spacing: 0.12em;
}

.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 6px 0;
}

.row__label {
  font-size: 13px;
}

.row__note {
  margin-top: 6px;
  color: var(--fg-muted);
  font-size: 11px;
  line-height: 1.7;
}

.input {
  margin-left: auto;
  padding: 5px 8px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: transparent;
  color: var(--fg);
  font-family: inherit;
  font-size: 13px;
}

.input[type='number'] {
  width: 72px;
  text-align: right;
}

.input[type='range'] {
  width: 180px;
}

.seg {
  display: flex;
  gap: 6px;
  margin-left: auto;
}

.seg__item {
  padding: 5px 14px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: transparent;
  color: var(--fg-muted);
  font-size: 12px;
}

.seg__item--on {
  border-color: var(--accent);
  color: var(--accent);
}

.link {
  margin-left: auto;
  border: none;
  background: transparent;
  color: var(--accent);
  font-size: 13px;
  text-decoration: underline;
}

.about__line {
  font-size: 13px;
}

.about__sub {
  margin-top: 10px;
  color: var(--fg-muted);
  font-size: 11px;
}

.limits {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 8px;
  color: var(--fg-muted);
  font-size: 11px;
  line-height: 1.7;
  list-style: none;
  padding: 0;
}

.limits li {
  padding-left: 12px;
  border-left: 1px solid var(--dialog-border);
}
</style>
