import { createFileRoute, Outlet } from '@tanstack/react-router'

import { DesktopSidebar } from '../components/Sidebar'

function DocsLayout() {
  return (
    <div className="relative mx-auto flex h-full w-full max-w-[1100px]">
      <DesktopSidebar />
      <main className="min-w-0 flex-1 overflow-y-auto px-4 py-10 sm:px-8 sm:py-12 md:px-10 md:py-14">
        <div className="mx-auto max-w-[680px] text-[0.875rem] leading-relaxed [&_article>h1]:mb-3 [&_article>h1]:text-2xl [&_article>h1]:font-normal [&_article>h1]:text-koma-fg sm:[&_article>h1]:text-3xl [&_article_h2]:mt-10 [&_article_h2]:mb-3 [&_article_h2]:text-lg [&_article_h2]:font-normal [&_article_h2]:text-koma-fg [&_article_p]:mb-4 [&_article_p]:leading-relaxed">
          <Outlet />
        </div>
      </main>
    </div>
  )
}

export const Route = createFileRoute('/_docs')({
  component: DocsLayout,
})
