import { createApp } from 'vue'
import { createPinia } from 'pinia'

import App from './App.vue'
import './styles/theme.css'

// Pinia 只保存 Rust 推来的投影（TDD §3.2），这里先建好容器。
createApp(App).use(createPinia()).mount('#app')
