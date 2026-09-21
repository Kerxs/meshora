/**
 * 滚动渐入 + 路由切换过渡。
 *
 * 这个文件里最重要的不是效果，是**失败时内容仍然可见**：
 *
 *   1. 「待入场」的隐藏类只在这里由 JS 添加。JS 没加载、没执行、或在此之前抛错，
 *      页面就是一个普通的、内容完全可见的静态页 —— 这对 SSG 站点和爬虫是底线。
 *   2. 不支持 IntersectionObserver、或者观察器构造失败，直接全部标记为已入场。
 *   3. 用户开了「减少动态效果」，整组跳过，连类都不加。
 */

/*
  顺序有意义：靠前的选择器先占位，靠后的如果落在已占位元素内部会被跳过
  （见下面的祖先检查）。所以首页的 .home-section 整块渐入，
  里面的架构图/流水线/表格不再各自渐入；文档页没有 .home-section，
  它们就各自渐入。
*/
const TARGETS = [
  '.VPHome .VPFeatures .item',
  '.home-section',
  '.vp-doc .mesh-diagram',
  '.vp-doc .pipeline',
  '.vp-doc table'
]

let observer: IntersectionObserver | undefined
let failsafe: number | undefined

export function teardownReveal() {
  observer?.disconnect()
  observer = undefined
  if (failsafe !== undefined) {
    clearTimeout(failsafe)
    failsafe = undefined
  }
  document.removeEventListener('visibilitychange', rescueVisible)
}

/**
 * 兜底：把「已经在视口里、却还没入场」的元素强制显示。
 *
 * 文档处于隐藏状态（后台标签页、未显示的预览面板）时，页面不做渲染，
 * IntersectionObserver 会认为所有元素都不相交，于是一个都不会入场。
 * 等文档重新可见时观察器通常会补发，但这条路径不值得赌 ——
 * 内容永久不可见是文档站最糟的失败模式。
 *
 * 只救视口内的元素，这样首屏之下的渐入效果不会被提前打光。
 */
function rescueVisible() {
  if (document.visibilityState !== 'visible') return
  document.querySelectorAll<HTMLElement>('.reveal:not(.reveal-in)').forEach(el => {
    const r = el.getBoundingClientRect()
    if (r.top < window.innerHeight && r.bottom > 0) el.classList.add('reveal-in')
  })
}

/**
 * 首屏入场动画的开关。
 *
 * 入场动画用的是 animation-fill-mode: both —— 动画没跑起来，元素就永久停在
 * opacity: 0。所以这个类只由 JS 添加：JS 没执行，CSS 里那些入场规则根本不匹配，
 * hero 就是一个完全可见的静态首屏。
 */
export function enableEntranceAnimations() {
  if (typeof document === 'undefined') return
  if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return
  document.documentElement.classList.add('meshora-animate')
}

export function setupReveal() {
  if (typeof window === 'undefined' || typeof document === 'undefined') return

  // 路由切换会重新调用本函数，先断掉上一页的观察器，否则会一页页累积
  teardownReveal()

  // 同一个元素可能被多个选择器命中，去重，并按各自分组算错峰序号。
  // 还要跳过「祖先已经在入场名单里」的元素：父子都渐入的话，两层 opacity
  // 会在过渡期间相乘，子元素出现得比预期更晚更突兀。
  const seen = new Set<HTMLElement>()
  const groups = TARGETS.map(sel => {
    const out: HTMLElement[] = []
    document.querySelectorAll<HTMLElement>(sel).forEach(el => {
      if (seen.has(el)) return
      for (const other of seen) {
        if (other !== el && other.contains(el)) return
      }
      seen.add(el)
      out.push(el)
    })
    return out
  })

  const all = groups.flat()
  if (!all.length) return

  const revealAll = () => all.forEach(el => el.classList.add('reveal-in'))

  const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  if (reduce || !('IntersectionObserver' in window)) {
    revealAll()
    return
  }

  try {
    const io = new IntersectionObserver(
      entries => {
        for (const e of entries) {
          if (!e.isIntersecting) continue
          e.target.classList.add('reveal-in')
          io.unobserve(e.target) // 入场一次就够，不做来回进出的重复动画
        }
      },
      { rootMargin: '0px 0px -8% 0px', threshold: 0.05 }
    )

    groups.forEach(group => {
      group.forEach((el, i) => {
        // 错峰上限 8，避免长列表末尾等太久
        el.style.setProperty('--reveal-i', String(Math.min(i, 8)))
        el.classList.add('reveal')
      })
    })

    all.forEach(el => io.observe(el))
    observer = io

    // 两道兜底：文档转为可见时补一次，以及 2.5 秒后无条件补一次
    document.addEventListener('visibilitychange', rescueVisible)
    failsafe = window.setTimeout(rescueVisible, 2500)
  } catch {
    // 观察器没建起来就别把内容藏着
    all.forEach(el => el.classList.remove('reveal'))
    revealAll()
  }
}

/**
 * 路由切换后重放正文淡入。
 * 必须「移除类 → 强制重排 → 加回类」，否则同一个动画不会第二次播放。
 */
export function playPageEnter() {
  const el = document.querySelector<HTMLElement>('.VPContent')
  if (!el) return
  el.classList.remove('page-enter')
  void el.offsetWidth // 强制重排，这一行不能删
  el.classList.add('page-enter')
}
