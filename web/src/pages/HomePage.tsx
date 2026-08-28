import { FlaskConical } from 'lucide-react'

import { Button } from '@/components/ui/button'

function HomePage() {
  return (
    <main className="min-h-screen bg-background text-foreground">
      <section className="mx-auto flex min-h-screen w-full max-w-5xl items-center px-6 py-16">
        <div className="space-y-4">
          <Button
            aria-label="Agentic Lab"
            className="pointer-events-none"
            size="icon-lg"
            variant="outline"
          >
            <FlaskConical className="h-5 w-5" aria-hidden="true" />
          </Button>
          <div className="space-y-2">
            <h1 className="text-3xl font-semibold tracking-normal text-balance">
              Agentic Lab
            </h1>
            <p className="max-w-xl text-base leading-7 text-muted-foreground">
              Frontend setup complete.
            </p>
          </div>
        </div>
      </section>
    </main>
  )
}

export default HomePage
