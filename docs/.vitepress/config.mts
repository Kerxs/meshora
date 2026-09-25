import { defineConfig } from 'vitepress'

/*
  站点部署在 GitHub Pages 的项目子路径下。
  换成自定义域名时要同时改三处：SITE、下面的 base（改回 '/'），
  并在 docs/public/ 下放回 CNAME 文件。
*/
const SITE = 'https://kerxs.github.io/meshora'

/**
 * MiniSearch 默认按空白和标点分词，整段中文会变成一个 token，本地搜索几乎搜不到东西。
 * 这里对中日韩文字同时产出单字和双字 token：单字保召回，双字（"中继"、"打洞"）保精度。
 * 索引和查询走的是同一个 tokenize，所以两边行为一致。
 */
function tokenize(text: string): string[] {
  const out: string[] = []

  const latin = text.toLowerCase().match(/[a-z0-9_.+-]+/g)
  if (latin) out.push(...latin)

  const cjkRuns = text.match(/[㐀-䶿一-鿿぀-ヿ가-힯]+/g)
  if (cjkRuns) {
    for (const run of cjkRuns) {
      for (let i = 0; i < run.length; i++) {
        out.push(run[i])
        if (i + 1 < run.length) out.push(run.slice(i, i + 2))
      }
    }
  }

  return out
}

export default defineConfig({
  title: 'Meshora',
  description: '开源、跨平台的设备网络连接与能力编排平台',
  lang: 'zh-CN',
  // 部署到 user.github.io/repo/ 时 base 必须是 /repo/
  base: '/meshora/',

  // GitHub Pages 原生支持「不重定向地把 /foo 当 /foo.html 提供」，
  // 所以这里可以开着（VitePress 路由文档点名 GitHub Pages 默认支持）。
  cleanUrls: true,

  // 现在只有中文，但先摆成 locales 结构：
  // 以后补英文只需加 docs/en/ 和一个 en 条目，不用重排任何链接。
  locales: {
    root: { label: '简体中文', lang: 'zh-CN' }
  },

  /*
    末尾的斜杠不能省。sitemap 是用 new URL(页面路径, hostname) 拼的，
    hostname 末尾没有斜杠时，相对路径会替换掉最后一段 ——
    结果是 https://kerxs.github.io/guide/x，/meshora 被吃掉了。
  */
  sitemap: { hostname: SITE + '/' },

  head: [
    ['link', { rel: 'preconnect', href: 'https://fonts.googleapis.com' }],
    ['link', { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: '' }],
    ['link', {
      rel: 'stylesheet',
      href: 'https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600&family=IBM+Plex+Mono:wght@400;500&display=swap'
    }],
    ['meta', { name: 'theme-color', content: '#0E5C63' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:site_name', content: 'Meshora' }],
    ['meta', { property: 'og:title', content: 'Meshora — Connect Everything.' }],
    ['meta', { property: 'og:description', content: '开源、跨平台的设备网络连接与能力编排平台。目前处于设计阶段。' }],
    ['meta', { property: 'og:url', content: SITE }]
  ],

  themeConfig: {
    logo: undefined,
    siteTitle: 'Meshora',

    nav: [
      { text: '指南', link: '/guide/what-is-meshora', activeMatch: '/guide/' },
      { text: '架构', link: '/guide/architecture' },
      { text: '路线图', link: '/guide/roadmap' },
      { text: '参与', link: '/guide/contributing' }
    ],

    sidebar: {
      '/guide/': [
        {
          text: '认识 Meshora',
          items: [
            { text: '它解决什么问题', link: '/guide/what-is-meshora' },
            { text: '核心概念', link: '/guide/concepts' }
          ]
        },
        {
          text: '设计',
          items: [
            { text: '架构：控制面与数据面', link: '/guide/architecture' },
            { text: '自动网络：建连流水线', link: '/guide/connection-flow' },
            { text: '六大能力', link: '/guide/capabilities' },
            { text: '接口契约', link: '/guide/interfaces' },
            { text: '威胁模型', link: '/guide/threat-model' }
          ]
        },
        {
          text: '参考',
          items: [
            { text: '与同类方案对比', link: '/guide/comparison' },
            { text: '路线图', link: '/guide/roadmap' },
            { text: '常见问题', link: '/guide/faq' },
            { text: '参与进来', link: '/guide/contributing' }
          ]
        }
      ]
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/Kerxs/meshora' }
    ],

    search: {
      provider: 'local',
      options: {
        miniSearch: {
          options: { tokenize },
          searchOptions: { prefix: true, fuzzy: 0.2, boost: { title: 4, text: 2, titles: 1 } }
        },
        translations: {
          button: { buttonText: '搜索文档', buttonAriaLabel: '搜索文档' },
          modal: {
            displayDetails: '展开详情',
            resetButtonTitle: '清除',
            noResultsText: '没有找到结果',
            footer: { selectText: '选择', navigateText: '切换', closeText: '关闭' }
          }
        }
      }
    },

    outline: { level: [2, 3], label: '本页目录' },
    docFooter: { prev: '上一页', next: '下一页' },
    returnToTopLabel: '回到顶部',
    sidebarMenuLabel: '目录',
    darkModeSwitchLabel: '主题',
    lightModeSwitchTitle: '切换到浅色',
    darkModeSwitchTitle: '切换到深色',

    footer: {
      message: '以 MIT 协议发布 · 首页背景由 <a href="https://shaders.paper.design">Paper Shaders</a> 驱动（Apache-2.0）',
      copyright: 'Copyright © 2026 Kerxs'
    }
  }
})
