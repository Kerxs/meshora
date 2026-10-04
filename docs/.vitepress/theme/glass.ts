/**
 * 站点的磨砂玻璃：Glassium（https://github.com/Kerxs/glassium），和 Meshora 客户端是同一套。
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

/**
 * 窄屏上侧边栏是盖在正文上的抽屉：玻璃盖不住正文的字（会透上来），那时改用 custom.css 里的 CSS 毛玻璃。
 * 和 VitePress 切换抽屉的断点一致
 */
const WIDE = '(min-width: 960px)'

/** 三步联机在这个宽度以下变成横向滑动的卡片：玻璃跟不上容器内的横滑，交给 HomeSteps.vue 的 CSS 毛玻璃 */
const STEPS_GRID = '(min-width: 901px)'

/**
 * 触屏设备上一律不用 Glassium：它的玻璃画在页面底下的画布上、每帧按元素位置重画，手机的滚动由合成线程直接做、
 * 比主线程快一两帧，玻璃就落在文字后面。这些设备上由 custom.css 的 CSS 毛玻璃（backdrop-filter）顶上，和滚动同步
 */
export const TOUCH = '(hover: none) and (pointer: coarse)'

/** 选择器 → 材质；第三项是只在什么屏宽下才用玻璃 */
const RULES: [string, Record<string, string>, string?][] = [
  ['.VPSidebar', { glass: 'frosted' }, WIDE],
  // 文档正文：一整块偏暗的磨砂玻璃，字要有一块稳的底
  ['.VPDoc .content-container', { glass: 'frosted', 'glass-tint': 'rgba(10, 12, 18, 0.72)', 'glass-blur': '40' }],
  ['.VPButton.brand', { glass: 'tinted', 'glass-tint': '#3d6bff' }],
  ['.VPButton.alt', { glass: 'clear' }],
  ['.vp-doc div[class*="language-"]', { glass: 'clear' }],
  ['.vp-doc .custom-block', { glass: 'clear' }],
  ['.VPDocAsideOutline', { glass: 'clear' }],
  ['.home-section', { glass: 'frosted' }],
  ['.steps .step', { glass: 'frosted' }, STEPS_GRID]
]

function mark(root: ParentNode) {
  for (const [selector, attrs, media] of RULES) {
    const wanted = !media || window.matchMedia(media).matches
    root.querySelectorAll<HTMLElement>(selector).forEach((el) => {
      if (!wanted) {
        // 屏宽变窄了：摘掉玻璃，交给 CSS
        for (const name of Object.keys(attrs)) el.removeAttribute(name)
        return
      }
      if (el.hasAttribute('glass')) return
      for (const [name, value] of Object.entries(attrs)) el.setAttribute(name, value)
    })
  }
}

let observer: MutationObserver | null = null

export async function startGlass() {
  if (typeof window === 'undefined' || observer) return
  if (window.matchMedia(TOUCH).matches) return
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
  // 跨过断点（旋转屏幕、拖窗口）时重新标一遍
  for (const media of new Set(RULES.map(([, , m]) => m).filter(Boolean) as string[])) {
    window.matchMedia(media).addEventListener('change', () => mark(document))
  }
}
