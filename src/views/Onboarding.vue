<script setup lang="ts">
// 首次引导（PRD F-11）：三屏，必须逐屏确认，不接受"跳过"。
// 文案口径来自 PRD §11：诚实到底——拦不住的说拦不住，别承诺做不到的事。
import { computed, ref } from 'vue'

import { useSessionStore } from '../stores/session'

const store = useSessionStore()

const screens = computed(() => [
  {
    title: '先说清楚它是什么',
    lines: [
      '这是给你自己用的工具：设定时长，入关后电脑被全屏接管。',
      '它防的是"手一滑就想走"，不是防别人，也不防你自己关机。',
      '全程离线：不联网、不登录、不收集任何数据。',
    ],
  },
  {
    // 这一屏**只说原则，不列具体做法**：把逃课的门路写进引导，等于教人怎么走。
    // 完整清单在「设置 → 关于」里，想看的人自己去看（2026-10-21 你的建议）。
    title: '它做不到什么',
    lines: [
      '它管得住这台电脑上的常规操作，但系统层面的东西不归它管。',
      '它也不会假装比你更坚定：真想走的人，总能找到办法。',
      '完整说明在「设置 → 关于」里，写着它拦不住什么。',
    ],
  },
  {
    title: '它怎么运作',
    lines: [
      '想提前出关：点「解开阵法」→ 等冷却 → 答问阵 → 全对方可出关。',
      '答错一次，冷却与题数都会涨；第 3 次起还要抄写。',
      '遇真急事：长按 Esc 八秒，抄九个字即可破戒——记为一次破戒，不计入累计。',
    ],
  },
])

const index = ref(0)
const last = computed(() => index.value >= screens.value.length - 1)

async function next(): Promise<void> {
  if (!last.value) {
    index.value += 1
    return
  }
  await store.finishOnboarding()
}
</script>

<template>
  <main class="onboard">
    <p class="onboard__step num">{{ index + 1 }} / {{ screens.length }}</p>
    <h1 class="onboard__title">{{ screens[index]?.title }}</h1>

    <ul class="onboard__lines">
      <li v-for="line in screens[index]?.lines" :key="line">{{ line }}</li>
    </ul>

    <button class="onboard__next" type="button" @click="next">
      {{ last ? '知道了，开始' : '继续' }}
    </button>
    <p class="onboard__hint">三步看完才能入关（没有「跳过」——这也是它的一部分）</p>
  </main>
</template>

<style scoped>
.onboard {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  justify-content: center;
  gap: 16px;
  width: min(560px, 100%);
  height: 100%;
  margin: 0 auto;
  padding: 32px 24px;
}

.onboard__step {
  color: var(--fg-muted);
  font-size: 12px;
  letter-spacing: 0.2em;
}

.onboard__title {
  font-size: 22px;
  font-weight: 500;
  letter-spacing: 0.04em;
}

.onboard__lines {
  display: flex;
  flex-direction: column;
  gap: 12px;
  color: var(--fg);
  font-size: 14px;
  line-height: 1.9;
  list-style: none;
  padding: 0;
}

.onboard__lines li {
  padding-left: 14px;
  border-left: 2px solid var(--dialog-border);
}

.onboard__next {
  margin-top: 8px;
  padding: 10px 26px;
  border: 1px solid var(--accent);
  border-radius: var(--radius-btn);
  background: var(--accent);
  color: #14110e;
  font-size: 14px;
  letter-spacing: 0.1em;
}

.onboard__hint {
  color: var(--fg-muted);
  font-size: 11px;
}
</style>
