import { createFileRoute } from '@tanstack/react-router'

function HomePage() {
  return (
    <div className="flex h-full items-center justify-center">
      <div className="text-center">
        <h1 className="mb-4 text-4xl font-bold text-koma-accent">koma</h1>
        <p className="mb-8 text-lg text-koma-dim">
          an agent that reads your repo, plans, edits, and runs.
        </p>
        <a
          href="#/docs/getting-started"
          className="inline-block rounded bg-koma-accent px-6 py-2 font-semibold text-koma-bg transition hover:opacity-80"
        >
          Get Started
        </a>
      </div>
    </div>
  )
}

export const Route = createFileRoute('/')({
  component: HomePage,
})
