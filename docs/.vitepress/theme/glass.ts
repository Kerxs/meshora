/**
 * 站点的玻璃：全部是 Glassium（https://github.com/Kerxs/glassium），和 Meshora 客户端是同一套。
 *
 * Glassium 接管页面上任何时候出现的 `[glass]`。VitePress 的默认主题是现成的组件，没法在模板里写属性，
 * 所以这里按选择器给它们标上（RULES）。换页、展开菜单时新出现的元素由 MutationObserver 补上。
 *
 * 浮在滚动的正文上面的（顶部导航、手机上的局部导航、"本页目录"下拉、搜索弹窗、窄屏的侧边栏抽屉）标 `overlay`：
 * Glassium 的 GPU 玻璃画在页面底下的画布上，盖不住压在它上面的字；overlay 的玻璃由 Glassium 照同一套材质用 CSS 画
 * （backdrop-filter，没有折射），盖得住。
 *
 * 只在浏览器里跑（onMounted 之后）：服务端渲染出来的是普通的 DOM，水合之后才变成玻璃。
 * 模板里别直接写 `glass` 属性：Glassium 一 import 就接管页面上已有的 [glass]，那时这里还没 configure。
 */

/** 和 VitePress 切换侧边栏抽屉的断点一致：窄于它时侧边栏是盖在正文上的抽屉 */
const WIDE = '(min-width: 960px)'

/** 三步联机在这个宽度以下变成横向滑动的卡片：GPU 玻璃跟不上容器里的横滑，改用 CSS 画（overlay） */
const STEPS_GRID = '(min-width: 901px)'

/**
 * 触屏设备（手机、平板）上 Glassium 用 CSS 画玻璃（`backend: 'css'`，和 Meshora 的安卓客户端一样）：GPU 玻璃画在
 * 页面底下的画布上、每帧按元素位置重画，手机的滚动由合成线程直接做、比主线程快一两帧，玻璃就落在文字后面；
 * CSS 画的（backdrop-filter）和滚动一起走。材质还是 Glassium 的，只是没有折射
 */
export const TOUCH = '(hover: none) and (pointer: coarse)'

/** 浮在正文上的玻璃要压得住下面的字：底色深一些 */
const FLOATING_TINT = 'rgba(12, 14, 20, 0.72)'

interface Rule {
  selector: string
  attrs: Record<string, string>
  /** 不满足这个媒体查询时改用 overlay（CSS 画）、换上 narrowAttrs */
  wideOnly?: string
  narrowAttrs?: Record<string, string>
  /** 触屏上不用玻璃 */
  noTouch?: boolean
}

const RULES: Rule[] = [
  // 浮在正文上的
  { selector: '.VPNavBar', attrs: { glass: 'frosted', 'glass-tint': FLOATING_TINT, overlay: '' } },
  { selector: '.VPLocalNav', attrs: { glass: 'frosted', 'glass-tint': FLOATING_TINT, overlay: '' } },
  // 下拉写在局部导航里面，而局部导航自己也是 CSS 玻璃：展开时 custom.css 关掉局部导航的模糊（见那里）
  { selector: '.VPLocalNavOutlineDropdown .items', attrs: { glass: 'frosted', 'glass-tint': 'rgba(16, 18, 26, 0.88)', overlay: '' } },
  { selector: '.VPNavScreen', attrs: { glass: 'frosted', 'glass-tint': 'rgba(7, 8, 12, 0.9)', overlay: '' } },
  { selector: '.VPLocalSearchBox .shell', attrs: { glass: 'frosted', 'glass-tint': 'rgba(16, 18, 26, 0.82)', overlay: '' } },
  // 侧边栏：宽屏上是 GPU 玻璃；窄屏上是盖在正文上的抽屉，改成 overlay、底色深一些
  {
    selector: '.VPSidebar',
    attrs: { glass: 'frosted' },
    wideOnly: WIDE,
    narrowAttrs: { glass: 'frosted', 'glass-tint': 'rgba(12, 14, 20, 0.9)', overlay: '' }
  },
  // 文档正文：一整块偏暗的磨砂玻璃，字要有一块稳的底。触屏上不用：面板比一屏高得多，backdrop-filter 铺这么大一块
  // 又费电、又可能超出 GPU 纹理上限只画出一截 —— custom.css 给它一层够实的底
  { selector: '.VPDoc .content-container', attrs: { glass: 'frosted', 'glass-tint': 'rgba(10, 12, 18, 0.72)', 'glass-blur': '40' }, noTouch: true },
  { selector: '.VPButton.brand', attrs: { glass: 'tinted', 'glass-tint': '#3d6bff' } },
  { selector: '.VPButton.alt', attrs: { glass: 'clear' } },
  { selector: '.vp-doc div[class*="language-"]', attrs: { glass: 'clear' } },
  { selector: '.vp-doc .custom-block', attrs: { glass: 'clear' } },
  { selector: '.vp-doc table', attrs: { glass: 'clear' } },
  { selector: '.VPDocAsideOutline', attrs: { glass: 'clear' } },
  { selector: '.VPDocFooter .pager-link', attrs: { glass: 'clear' } },
  // 首页
  { selector: '.home-section', attrs: { glass: 'frosted' } },
  { selector: '.hero .status', attrs: { glass: 'clear' } },
  {
    selector: 'section.steps .step',
    attrs: { glass: 'frosted' },
    wideOnly: STEPS_GRID,
    narrowAttrs: { glass: 'frosted', overlay: '' }
  },
  // 文档里的组件：建连流水线的步骤、使用场景的卡片和标签
  { selector: '.pipeline .chip', attrs: { glass: 'clear' } },
  { selector: '.scenarios .scenario', attrs: { glass: 'clear' } },
  { selector: '.scenarios .cap', attrs: { glass: 'clear' } }
]

/** 规则里用到的所有属性名：换一套之前先全摘掉 */
const NAMES = [...new Set(RULES.flatMap((r) => [...Object.keys(r.attrs), ...Object.keys(r.narrowAttrs ?? {})]))]

function wantedAttrs(rule: Rule, touch: boolean): Record<string, string> | null {
  if (touch && rule.noTouch) return null
  if (!rule.wideOnly || window.matchMedia(rule.wideOnly).matches) return rule.attrs
  return rule.narrowAttrs ?? null
}

function mark(root: ParentNode) {
  const touch = window.matchMedia(TOUCH).matches
  for (const rule of RULES) {
    const attrs = wantedAttrs(rule, touch)
    const key = attrs ? JSON.stringify(attrs) : ''
    root.querySelectorAll<HTMLElement>(rule.selector).forEach((el) => {
      if (el.dataset.glassRule === key) return
      el.dataset.glassRule = key
      for (const name of NAMES) el.removeAttribute(name)
      if (attrs) for (const [name, value] of Object.entries(attrs)) el.setAttribute(name, value)
    })
  }
}

let observer: MutationObserver | null = null

export async function startGlass() {
  if (typeof window === 'undefined' || observer) return
  const { default: glassium } = await import('glassium')
  // 玻璃后面那些写了背景的祖先（VitePress 的容器），也收进场景。触屏上不建 GPU 画布，玻璃用 CSS 画
  glassium.configure(window.matchMedia(TOUCH).matches ? { backend: 'css' } : { absorbForComponents: true })
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
  for (const media of new Set(RULES.map((r) => r.wideOnly).filter(Boolean) as string[])) {
    window.matchMedia(media).addEventListener('change', () => mark(document))
  }
}
