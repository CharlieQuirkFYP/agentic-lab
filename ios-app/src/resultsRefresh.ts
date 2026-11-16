/** Coalesce archive notifications without owning collection or native work. */
export function subscribeResultsRefresh(options: {
  subscribe: (notify: () => void) => () => void;
  refresh: () => void;
  now?: () => number;
  schedule?: (callback: () => void, delay: number) => ReturnType<typeof setTimeout>;
  cancel?: (timer: ReturnType<typeof setTimeout>) => void;
}) {
  const now = options.now ?? Date.now;
  const schedule = options.schedule ?? setTimeout;
  const cancel = options.cancel ?? clearTimeout;
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let last = -Infinity;
  const refresh = () => {
    timer = undefined;
    if (stopped) return;
    last = now();
    options.refresh();
  };
  const notify = () => {
    if (stopped || timer !== undefined) return;
    const delay = Math.max(0, 1000 - (now() - last));
    timer = schedule(refresh, delay);
  };
  const unsubscribe = options.subscribe(notify);
  refresh();
  return () => {
    stopped = true;
    unsubscribe();
    if (timer !== undefined) cancel(timer);
  };
}
