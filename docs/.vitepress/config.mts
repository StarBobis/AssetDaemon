import { defineConfig } from 'vitepress'

// https://vitepress.dev/reference/site-config
export default defineConfig({
  base: '/AssetDaemon/',
  title: 'AssetDaemon',
  description: 'Documents For AssetDaemon',
  themeConfig: {
    // https://vitepress.dev/reference/default-theme-config
    nav: [
      { text: 'Home', link: '/' },
      { text: '新手教程', link: '/tutorial/Introduction/Introduction' },
      { text: 'LTS', link: '/lts/Introduction/Introduction' }
    ],

    sidebar: [
      {
        text: '新手教程',
        items: [
          { text: '教程介绍', link: '/tutorial/Introduction/Introduction' }
        ]
      },
      {
        text: 'LTS',
        items: [
          { text: 'LTS 介绍', link: '/lts/Introduction/Introduction' }
        ]
      }
    ]
  }
})
