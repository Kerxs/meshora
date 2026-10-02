/**
 * 官网的开场动画，和 Meshora 客户端打开时同一套：点阵从中心一圈圈亮起（DotBackdrop），中间一笔一笔画出 Logo，
 * Logo 缩小、飞到左上角导航栏的 Logo 上，页面浮上来。
 *
 * 每个浏览器会话只放一次（sessionStorage），换页不放；系统要求减少动画、页面一开始就看不见时不放。
 * 要不要放在 config.mts 的 head 里那段内联脚本决定：它在第一帧之前给 <html> 加上 site-booting，
 * 页面内容先藏着，不会先闪一下再藏。它还设了一个 4 秒的兜底：这里的 JS 没跑起来，内容照样会出来。
 */

const KEY = 'meshora-intro'
const SVG_NS = 'http://www.w3.org/2000/svg'

/** 这次要不要放（head 里的脚本已经判断过，看它加没加 class） */
export function introPending(): boolean {
  return typeof document !== 'undefined' && document.documentElement.classList.contains('site-booting')
}

function drawnLogo(): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, 'svg')
  svg.setAttribute('viewBox', '0 0 64 64')
  svg.setAttribute('class', 'intro-logo')
  svg.setAttribute('aria-hidden', 'true')
  const g = document.createElementNS(SVG_NS, 'g')
  g.setAttribute('stroke', 'rgba(255,255,255,.6)')
  g.setAttribute('stroke-width', '3.2')
  g.setAttribute('stroke-linecap', 'round')
  const lines = [
    [18, 20, 46, 17],
    [18, 20, 20, 46],
    [18, 20, 45, 44],
    [46, 17, 20, 46],
    [46, 17, 45, 44],
    [20, 46, 45, 44]
  ]
  lines.forEach(([x1, y1, x2, y2], i) => {
    const line = document.createElementNS(SVG_NS, 'line')
    for (const [k, v] of Object.entries({ x1, y1, x2, y2, pathLength: 1 })) line.setAttribute(k, String(v))
    line.setAttribute('class', 'l')
    line.style.setProperty('--i', String(i))
    g.append(line)
  })
  svg.append(g)
  const nodes: [number, number, string][] = [
    [18, 20, '#fff'],
    [46, 17, '#fff'],
    [20, 46, '#fff'],
    [45, 44, '#5ff0c0']
  ]
  nodes.forEach(([cx, cy, fill], i) => {
    const c = document.createElementNS(SVG_NS, 'circle')
    c.setAttribute('cx', String(cx))
    c.setAttribute('cy', String(cy))
    c.setAttribute('r', '6.5')
    c.setAttribute('fill', fill)
    c.setAttribute('class', 'n')
    c.style.setProperty('--i', String(i))
    svg.append(c)
  })
  return svg
}

const wait = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

/** 放开场动画；放完调 `done`（接着放页面的入场动画） */
export async function runIntro(done: () => void) {
  if (!introPending()) return
  const root = document.documentElement
  try {
    sessionStorage.setItem(KEY, '1')
  } catch {}
  const layer = document.createElement('div')
  layer.className = 'site-intro'
  const mark = drawnLogo()
  layer.append(mark)
  document.body.append(layer)

  // Logo 画完（和客户端一样的节奏）
  await wait(1350)
  const target = document.querySelector('.VPNavBarTitle .logo')?.getBoundingClientRect()
  const from = mark.getBoundingClientRect()
  if (target && target.width > 0) {
    const dx = target.left + target.width / 2 - (from.left + from.width / 2)
    const dy = target.top + target.height / 2 - (from.top + from.height / 2)
    mark.style.setProperty('--fly', `translate(${dx}px, ${dy}px) scale(${target.width / from.width})`)
    layer.classList.add('fly')
  } else {
    layer.classList.add('fade')
  }
  root.classList.remove('site-booting')
  root.classList.add('site-booted')
  done()
  await wait(780)
  layer.remove()
  setTimeout(() => root.classList.remove('site-booted'), 1200)
}
