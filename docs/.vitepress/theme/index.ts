import { h, nextTick, onMounted, onUnmounted, watch } from 'vue'
import type { Theme } from 'vitepress'
import { useRoute } from 'vitepress'
import DefaultTheme from 'vitepress/theme'
import FluidBackdrop from './FluidBackdrop.vue'
import StatusBanner from './StatusBanner.vue'
import MeshDiagram from './MeshDiagram.vue'
import Pipeline from './Pipeline.vue'
import HomeSection from './HomeSection.vue'
import ScenarioGrid from './ScenarioGrid.vue'
import { enableEntranceAnimations, playPageEnter, setupReveal, teardownReveal } from './reveal'
import './custom.css'

/**
 * 首页和文档页的流体强弱不同（见 custom.css 的 --meshora-scrim）。
 * CSS 里优先用 :root:has(.VPHome) 判断，服务端渲染出来就是对的，不会闪。
 * 这里的 JS 只是给不支持 :has() 的浏览器兜底。
 */
function markHome(isHome: boolean) {
  if (typeof document === 'undefined') return
  document.documentElement.classList.toggle('is-home', isHome)
}

export default {
  extends: DefaultTheme,

  Layout() {
    return h(DefaultTheme.Layout, null, {
      // 全站常驻的背景层：整个会话只有这一个 WebGL 上下文
      'layout-top': () => h(FluidBackdrop),
      'home-hero-info-before': () => h(StatusBanner)
    })
  },

  enhanceApp({ app }) {
    // 在 Markdown 里直接写这些标签
    app.component('MeshDiagram', MeshDiagram)
    app.component('Pipeline', Pipeline)
    app.component('HomeSection', HomeSection)
    app.component('ScenarioGrid', ScenarioGrid)
  },

  /**
   * setup() 跑在根组件的 setup 里，SSR 阶段也会执行 —— 所以入场相关的工作
   * 全部挂在 onMounted 上，等水合完成后再动 DOM。
   * 在 enhanceApp 里提前改服务端渲染出来的 DOM 会引发水合不匹配。
   */
  setup() {
    const route = useRoute()

    onMounted(() => {
      markHome(route.path === '/')
      enableEntranceAnimations()
      setupReveal()
    })

    watch(
      () => route.path,
      path => {
        markHome(path === '/')
        nextTick(() => {
          playPageEnter()
          setupReveal()
        })
      }
    )

    onUnmounted(teardownReveal)
  }
} satisfies Theme
