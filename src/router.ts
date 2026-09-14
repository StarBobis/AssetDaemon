import { createRouter, createWebHistory } from 'vue-router'

const routes = [
  { path: '/', name: 'Home', component: () => import('./Home.vue') },
  { path: '/decrypt', name: 'Decrypt', component: () => import('./pages/Decrypt.vue') },
  { path: '/settings', name: 'Settings', component: () => import('./pages/Settings.vue') },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
})

export default router
