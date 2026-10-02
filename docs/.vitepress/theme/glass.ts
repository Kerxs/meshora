/**
 * 站点的液态玻璃：Glassium（https://github.com/Kerxs/glassium），和 Meshora 客户端是同一套。
 *
 * Glassium 接管页面上任何时候出现的 `[glass]`。VitePress 的默认主题是现成的组件，没法在模板里写属性，
 * 所以这里按选择器给它们标上：侧边栏、首页的能力卡片和按钮、首页区块、文档里的代码块和提示框。
 *
 * 顶部导航、手机上的局部导航不用它：它们浮在滚动的正文上面，Glassium 的玻璃画在页面底下的画布上，
 * 盖不住压在它上面的字（Glassium 的文档也这么说：盖在正文上的玻璃用 CSS 画）。那两个在 custom.css 里是 CSS 毛玻璃。
 * 换页、展开菜单时新出现的元素由 MutationObserver 补上。
 *
 * 只在浏览器里跑（onMounted 之后）：服务端渲染出来的是普通的 DOM，水合之后才变成玻璃。
 */

/** 选择器 → 材质 */
const RULES: [string, Record<string, string>][] = [
  ['.VPSidebar', { glass: '' }],
  // 文档正文：一整块偏暗的玻璃，长文下面的光团在动，字要有一块稳的底
  ['.VPDoc .content-container', { glass: 'tinted', 'glass-tint': 'rgba(6, 10, 32, 0.72)', 'glass-blur': '40' }],
  ['.VPFeature', { glass: '' }],
  ['.VPButton.brand', { glass: 'tinted', 'glass-tint': '#3d6bff' }],
  ['.VPButton.alt', { glass: 'clear' }],
  ['.vp-doc div[class*="language-"]', { glass: 'clear' }],
  ['.vp-doc .custom-block', { glass: 'clear' }],
  ['.VPDocAsideOutline', { glass: 'clear' }],
  ['.home-section', { glass: '' }]
]

function mark(root: ParentNode) {
  for (const [selector, attrs] of RULES) {
    root.querySelectorAll<HTMLElement>(selector).forEach((el) => {
      if (el.hasAttribute('glass')) return
      for (const [name, value] of Object.entries(attrs)) el.setAttribute(name, value)
    })
  }
}

let observer: MutationObserver | null = null

export async function startGlass() {
  if (typeof window === 'undefined' || observer) return
  const { default: glassium } = await import('glassium')
  // 玻璃后面那些写了背景的祖先（VitePress 的容器），也收进场景
  glassium.configure({ absorbForComponents: true })
  mark(document)
  let queued = false
  observer = new MutationObserver(() => {
    if (queued) return
    queued = true
    // 一帧里多次变动只扫一遍
    setTimeout(() => {
      queued = false
      mark(document)
    }, 50)
  })
  observer.observe(document.body, { childList: true, subtree: true })
}
