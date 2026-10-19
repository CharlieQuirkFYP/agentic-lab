import { AudioLines, BarChart3, FlaskConical } from "lucide-react";
import { NavLink, Outlet, useLocation } from "react-router-dom";

import { useLocalSpeech } from "@/hooks/useLocalSpeech";
import { useMicrophone } from "@/hooks/useMicrophone";
import { useVoiceTurn } from "@/hooks/useVoiceTurn";

export interface ConsoleContext {
  voice: ReturnType<typeof useVoiceTurn>;
  microphone: ReturnType<typeof useMicrophone>;
  speech: ReturnType<typeof useLocalSpeech>;
}

export default function ConsoleLayout() {
  const { pathname } = useLocation();
  const speech = useLocalSpeech();
  const voice = useVoiceTurn((text) => {
    if (pathname === "/voice") speech.liveReply(text);
  });
  const microphone = useMicrophone((clip) => {
    void voice.start(clip);
  });
  const context: ConsoleContext = { voice, microphone, speech };

  return (
    <div className="min-h-screen bg-slate-50/70">
      <header className="border-b bg-background">
        <div className="mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-4 px-5 py-4 sm:px-8">
          <div className="flex items-center gap-3">
            <div className="rounded-lg border bg-slate-50 p-2">
              <FlaskConical className="size-5" aria-hidden="true" />
            </div>
            <div>
              <p className="text-sm font-semibold">Agentic Lab</p>
              <p className="text-xs text-muted-foreground">
                Local voice development · Pheme VA
              </p>
            </div>
          </div>
          <nav
            aria-label="Main navigation"
            className="flex gap-1 rounded-lg bg-muted/60 p-1 text-sm"
          >
            <NavLink
              to="/voice"
              className={({ isActive }) =>
                `flex items-center gap-2 rounded-md px-3 py-2 focus-visible:outline-2 ${isActive ? "bg-white font-medium shadow-xs" : "text-muted-foreground hover:text-foreground"}`
              }
            >
              <AudioLines className="size-4" aria-hidden="true" /> Voice console
            </NavLink>
            <NavLink
              to="/benchmarks"
              onClick={() => {
                microphone.discard();
                speech.stop();
              }}
              className={({ isActive }) =>
                `flex items-center gap-2 rounded-md px-3 py-2 focus-visible:outline-2 ${isActive ? "bg-white font-medium shadow-xs" : "text-muted-foreground hover:text-foreground"}`
              }
            >
              <BarChart3 className="size-4" aria-hidden="true" /> Benchmarks
            </NavLink>
          </nav>
        </div>
      </header>
      <Outlet context={context} />
      <footer className="mx-auto max-w-7xl px-5 py-6 text-xs leading-5 text-muted-foreground sm:px-8">
        Development console, not a finalized incident-reporting system. This
        server has one shared web conversation; the TUI can inspect and approve
        its pending question.
      </footer>
    </div>
  );
}
