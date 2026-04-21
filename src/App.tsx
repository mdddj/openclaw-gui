import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import "./App.css";

const GATEWAY_LOG_EVENT = "gateway-log";
const GATEWAY_STATE_EVENT = "gateway-state";

type GatewayStatus = "stopped" | "running";

type GatewayExitInfo = {
  atMs: number;
  code: number | null;
  success: boolean;
};

type GatewayLogEntry = {
  id: number;
  timestampMs: number;
  stream: string;
  message: string;
};

type GatewayStatePayload = {
  command: string;
  status: GatewayStatus;
  pid: number | null;
  startedAtMs: number | null;
  lastError: string | null;
  lastExit: GatewayExitInfo | null;
};

type GatewaySnapshot = GatewayStatePayload & {
  logs: GatewayLogEntry[];
};

type GatewayAction = "start_gateway" | "stop_gateway" | "restart_gateway";

const EMPTY_SNAPSHOT: GatewaySnapshot = {
  command: "openclaw gateway",
  status: "stopped",
  pid: null,
  startedAtMs: null,
  lastError: null,
  lastExit: null,
  logs: [],
};

function App() {
  const [snapshot, setSnapshot] = useState<GatewaySnapshot>(EMPTY_SNAPSHOT);
  const [isLoading, setIsLoading] = useState(true);
  const [pendingAction, setPendingAction] = useState<GatewayAction | null>(null);
  const [isOpeningDashboard, setIsOpeningDashboard] = useState(false);
  const [now, setNow] = useState(Date.now());
  const logViewportRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let active = true;
    const cleanup: UnlistenFn[] = [];

    async function bootstrap() {
      try {
        const unlistenLog = await listen<GatewayLogEntry>(GATEWAY_LOG_EVENT, (event) => {
          if (!active) {
            return;
          }

          setSnapshot((current) => ({
            ...current,
            logs: [...current.logs, event.payload].slice(-800),
          }));
        });

        const unlistenState = await listen<GatewayStatePayload>(
          GATEWAY_STATE_EVENT,
          (event) => {
            if (!active) {
              return;
            }

            setSnapshot((current) => ({
              ...current,
              ...event.payload,
            }));
          },
        );

        cleanup.push(unlistenLog, unlistenState);

        const initialSnapshot = await invoke<GatewaySnapshot>("get_gateway_snapshot");
        if (active) {
          setSnapshot(initialSnapshot);
        }
      } catch (error) {
        console.error("Failed to bootstrap gateway snapshot", error);
      } finally {
        if (active) {
          setIsLoading(false);
        }
      }
    }

    bootstrap();

    return () => {
      active = false;
      cleanup.forEach((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    logViewportRef.current?.scrollTo({
      top: logViewportRef.current.scrollHeight,
      behavior: "smooth",
    });
  }, [snapshot.logs.length]);

  async function runAction(action: GatewayAction) {
    setPendingAction(action);

    try {
      const nextSnapshot = await invoke<GatewaySnapshot>(action);
      setSnapshot(nextSnapshot);
    } catch (error) {
      console.error(`Failed to execute gateway action: ${action}`, error);
    } finally {
      setPendingAction(null);
    }
  }

  async function openDashboard() {
    setIsOpeningDashboard(true);

    try {
      await invoke("open_dashboard");
    } catch (error) {
      console.error("Failed to open dashboard", error);
    } finally {
      setIsOpeningDashboard(false);
    }
  }

  const uptimeLabel =
    snapshot.startedAtMs === null ? "未运行" : formatDuration(now - snapshot.startedAtMs);

  const isRunning = snapshot.status === "running";
  const controlsDisabled = pendingAction !== null;

  return (
    <main className="app-shell">
      <section className="topbar">
        <div className="control-panel">
          <div className="title-block">
            <h1>OpenClaw Gateway</h1>
          </div>

          <div className="control-main">
            <div className="control-summary">
              <div className="status-bar">
                <div className={`status-pill status-${snapshot.status}`}>
                  <span className="status-dot" />
                  {snapshot.status === "running" ? "运行中" : "已停止"}
                </div>
                <span className="status-meta">PID {snapshot.pid ?? "--"}</span>
                <span className="status-meta">运行时长 {uptimeLabel}</span>
                {isLoading ? <span className="panel-tag">初始化中</span> : null}
              </div>
            </div>

            <div className="actions">
              <button
                disabled={isOpeningDashboard}
                onClick={openDashboard}
                type="button"
              >
                {isOpeningDashboard ? "打开中..." : "打开控制台"}
              </button>
              <button
                className="primary"
                disabled={controlsDisabled || isRunning}
                onClick={() => runAction("start_gateway")}
                type="button"
              >
                {pendingAction === "start_gateway" ? "启动中..." : "启动"}
              </button>
              <button
                disabled={controlsDisabled || !isRunning}
                onClick={() => runAction("stop_gateway")}
                type="button"
              >
                {pendingAction === "stop_gateway" ? "停止中..." : "停止"}
              </button>
              <button
                disabled={controlsDisabled}
                onClick={() => runAction("restart_gateway")}
                type="button"
              >
                {pendingAction === "restart_gateway" ? "重启中..." : "重新启动"}
              </button>
            </div>
          </div>
        </div>
      </section>

      <section className="workspace">
        <div className="panel logs-panel">
          <div className="panel-header">
            <h2>实时日志</h2>
            <div className="log-header-meta">
              <span className="panel-tag">{snapshot.logs.length} 条</span>
              <span className="log-header-streams">stdout / stderr / system</span>
            </div>
          </div>

          <div className="log-viewport" ref={logViewportRef}>
            {snapshot.logs.length === 0 ? (
              <div className="log-empty">
                <strong>还没有日志输出</strong>
                <p>stdout / stderr / system 输出会实时显示在这里。</p>
              </div>
            ) : (
              snapshot.logs.map((entry) => (
                <article className="log-entry" key={entry.id}>
                  <span className={`log-stream log-${entry.stream}`}>{entry.stream}</span>
                  <time className="log-time">{formatTime(entry.timestampMs)}</time>
                  <code className="log-message">{entry.message}</code>
                </article>
              ))
            )}
          </div>
        </div>
      </section>
    </main>
  );
}

function formatTime(timestamp: number) {
  return new Intl.DateTimeFormat("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(timestamp);
}

function formatDuration(durationMs: number) {
  const totalSeconds = Math.max(0, Math.floor(durationMs / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return `${hours}h ${minutes}m ${seconds}s`;
  }
  if (minutes > 0) {
    return `${minutes}m ${seconds}s`;
  }
  return `${seconds}s`;
}

export default App;
