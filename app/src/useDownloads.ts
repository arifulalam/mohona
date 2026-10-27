import { useCallback, useEffect, useRef, useState } from "react";
import type { Api, EngineStatus, Plan, Row } from "./types";

export interface Downloads {
  rows: Row[];
  plan: Plan | null;
  engine: EngineStatus | null;
  /** The last error as text, until dismissed. */
  error: string | null;
  dismissError(): void;
  startEngine(): Promise<void>;
  /** Runs an action, shows its error, then refreshes the list. */
  act(action: () => Promise<unknown>): Promise<boolean>;
}

/** Keeps the list and the engine's state fresh (every `every` ms). */
export function useDownloads(api: Api, every = 1000): Downloads {
  const [rows, setRows] = useState<Row[]>([]);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [engine, setEngine] = useState<EngineStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const busy = useRef(false);

  const refresh = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    try {
      const status = await api.status();
      setEngine(status);
      if (status.running) {
        setRows(await api.list());
        setPlan(await api.plan());
      }
    } catch (e) {
      setEngine({ running: false, version: null, message: String((e as Error).message ?? e) });
    } finally {
      busy.current = false;
    }
  }, [api]);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), every);
    return () => clearInterval(timer);
  }, [refresh, every]);

  const act = useCallback(
    async (action: () => Promise<unknown>) => {
      try {
        await action();
        setError(null);
        await refresh();
        return true;
      } catch (e) {
        setError(String((e as Error).message ?? e));
        await refresh();
        return false;
      }
    },
    [refresh],
  );

  const startEngine = useCallback(async () => {
    await act(() => api.start());
  }, [act, api]);

  return { rows, plan, engine, error, dismissError: () => setError(null), startEngine, act };
}
