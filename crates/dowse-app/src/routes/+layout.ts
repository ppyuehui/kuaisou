// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode.
// 主窗口和独立索引进度窗口都加载同一份 index.html，由 SvelteKit 客户端路由
// 按 URL 路径（/ 与 /indexing）渲染对应页面。
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
export const ssr = false;
