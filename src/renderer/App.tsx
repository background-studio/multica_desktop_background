import {
  Check,
  ChevronDown,
  CirclePause,
  ExternalLink,
  Film,
  FolderOpen,
  Image as ImageIcon,
  Link,
  MonitorUp,
  Play,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Shuffle,
  SlidersHorizontal,
  Trash2,
  Upload,
  Video,
  X,
} from "lucide-react";
import {
  CSSProperties,
  PointerEvent as ReactPointerEvent,
  ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  AppSnapshot,
  DisplaySettings,
  FitMode,
  MediaItem,
  SettingsPatch,
} from "../shared/contracts";
import { bridge } from "./bridge";
import brandIcon from "./brand-icon.png";

type InspectorTab = "display" | "pages" | "slideshow" | "settings";
type PreviewRoute = "home" | "task";

const BUSY_INDICATOR_DELAY_MS = 300;

const fitLabels: Record<FitMode, string> = {
  cover: "覆盖",
  contain: "适应",
  fill: "拉伸",
  tile: "平铺",
};

function formatBytes(bytes: number) {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

function ActionButton({
  icon,
  children,
  tone = "default",
  disabled,
  onClick,
  title,
}: {
  icon: ReactNode;
  children: ReactNode;
  tone?: "default" | "primary" | "danger";
  disabled?: boolean;
  onClick?: () => void;
  title?: string;
}) {
  return (
    <button className={`action-button action-${tone}`} disabled={disabled} onClick={onClick} title={title}>
      {icon}<span>{children}</span>
    </button>
  );
}

function IconButton({ icon, label, onClick, active, danger }: {
  icon: ReactNode;
  label: string;
  onClick: () => void;
  active?: boolean;
  danger?: boolean;
}) {
  return (
    <button
      className={`icon-button${active ? " is-active" : ""}${danger ? " is-danger" : ""}`}
      onClick={(event) => { event.stopPropagation(); onClick(); }}
      aria-label={label}
      title={label}
    >
      {icon}
    </button>
  );
}

function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (value: boolean) => void; label: string }) {
  return (
    <label className="toggle-row">
      <span>{label}</span>
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
      <span className="toggle-track" aria-hidden="true"><span /></span>
    </label>
  );
}

function RangeControl({ label, value, min, max, step, suffix = "", onChange }: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  suffix?: string;
  onChange: (value: number) => void;
}) {
  const display = step < 0.1 ? value.toFixed(2) : Number.isInteger(value) ? value : value.toFixed(1);
  return (
    <label className="range-control">
      <span className="control-label"><span>{label}</span><output>{display}{suffix}</output></span>
      <input type="range" value={value} min={min} max={max} step={step} onChange={(event) => onChange(Number(event.target.value))} />
    </label>
  );
}

function MediaThumb({ item, active, inPlaylist, onActivate, onTogglePlaylist, onRefresh, onRemove }: {
  item: MediaItem;
  active: boolean;
  inPlaylist: boolean;
  onActivate: () => void;
  onTogglePlaylist: () => void;
  onRefresh: () => void;
  onRemove: () => void;
}) {
  return (
    <div
      className={`media-thumb${active ? " is-active" : ""}`}
      onClick={onActivate}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onActivate();
        }
      }}
      role="button"
      tabIndex={0}
      title={item.name}
    >
      <span className="thumb-visual">
        {item.kind === "video" ? (
          <video src={item.previewUrl} muted preload="metadata" />
        ) : (
          <img src={item.previewUrl} alt="" loading="lazy" />
        )}
        <span className="thumb-kind">{item.origin === "api" ? <Shuffle size={13} /> : item.origin === "folder" ? <FolderOpen size={13} /> : item.kind === "video" ? <Film size={13} /> : <ImageIcon size={13} />}</span>
        <span className="thumb-actions">
          {(item.origin === "api" || item.origin === "folder") && (
            <IconButton
              icon={<RefreshCw size={14} />}
              label={item.origin === "folder" ? "换一张（从文件夹重选）" : "换一张（重新请求 API）"}
              onClick={onRefresh}
            />
          )}
          <IconButton
            icon={inPlaylist ? <Check size={14} /> : <Plus size={14} />}
            label={inPlaylist ? "从轮播移除" : "加入轮播"}
            active={inPlaylist}
            onClick={onTogglePlaylist}
          />
          <IconButton icon={<Trash2 size={14} />} label="删除媒体" danger onClick={onRemove} />
        </span>
      </span>
      <span className="thumb-name">{item.name}</span>
      <span className="thumb-meta">{item.origin === "api" ? "随机 API" : item.origin === "folder" ? `文件夹 · ${item.fileCount ?? 0} 个` : item.origin === "remote" ? "网络媒体" : item.kind === "video" ? "视频" : "图片"}{item.origin === "folder" ? "" : ` · ${formatBytes(item.byteSize)}`}</span>
    </div>
  );
}

function Preview({ item, display, route, onRouteChange, onPositionChange }: {
  item: MediaItem | null;
  display: DisplaySettings;
  route: PreviewRoute;
  onRouteChange: (route: PreviewRoute) => void;
  onPositionChange: (x: number, y: number) => void;
}) {
  const intensity = route === "home" ? display.homeIntensity : display.taskIntensity;
  const enabled = route === "home" ? display.enabledOnHome : display.enabledOnTasks;
  const previewStyle = {
    "--preview-opacity": display.opacity * intensity * (enabled ? 1 : 0),
    "--preview-blur": `${display.blur}px`,
    "--preview-scale": display.scale,
    "--preview-fit": display.fit === "tile" ? "cover" : display.fit,
    "--preview-x": `${display.positionX}%`,
    "--preview-y": `${display.positionY}%`,
    "--preview-overlay": display.overlayColor,
    "--preview-overlay-opacity": display.overlayOpacity,
    "--preview-sidebar": display.sidebarOpacity,
    "--preview-surface": display.surfaceOpacity,
    "--preview-composer": display.composerOpacity,
    "--preview-url": `url("${item?.previewUrl ?? ""}")`,
  } as CSSProperties;

  const updatePosition = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!item) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const x = Math.round(Math.min(100, Math.max(0, ((event.clientX - rect.left) / rect.width) * 100)));
    const y = Math.round(Math.min(100, Math.max(0, ((event.clientY - rect.top) / rect.height) * 100)));
    onPositionChange(x, y);
  };

  return (
    <section className="preview-section">
      <div className="preview-toolbar">
        <div className="segmented" aria-label="预览页面">
          <button className={route === "home" ? "is-active" : ""} onClick={() => onRouteChange("home")}>页面</button>
          <button className={route === "task" ? "is-active" : ""} onClick={() => onRouteChange("task")}>空白页</button>
        </div>
        <span className="preview-hint">拖动预览调整焦点</span>
      </div>
      <div
        className={`stage${display.fit === "tile" && item?.kind === "image" ? " is-tiled" : ""}`}
        style={previewStyle}
        onPointerDown={(event) => { event.currentTarget.setPointerCapture(event.pointerId); updatePosition(event); }}
        onPointerMove={(event) => { if (event.currentTarget.hasPointerCapture(event.pointerId)) updatePosition(event); }}
      >
        {item ? (
          <>
            <div className="stage-media-wrap">
              {item.kind === "video" ? (
                <video src={item.previewUrl} autoPlay loop muted={display.videoMuted} playsInline />
              ) : (
                <img src={item.previewUrl} alt="当前背景预览" draggable={false} />
              )}
            </div>
            <div className="stage-overlay" />
            <div className="stage-sidebar"><span /><span /><span /><span /></div>
            <div className={`stage-surface route-${route}`}>
              {route === "home" ? (
                <div className="home-composition"><i /><i /><i /></div>
              ) : (
                <div className="task-composition"><i /><i /><i /><i /></div>
              )}
              <div className="composer-preview"><span /></div>
            </div>
            <span className="focus-marker" style={{ left: `${display.positionX}%`, top: `${display.positionY}%` }} />
          </>
        ) : (
          <div className="stage-empty"><ImageIcon size={34} /><strong>选择一个背景</strong></div>
        )}
      </div>
      <div className="selected-summary">
        <span className="selected-icon">{item?.kind === "video" ? <Video size={18} /> : <ImageIcon size={18} />}</span>
        <span><strong>{item?.name ?? "未选择媒体"}</strong><small>{item ? `${item.origin === "api" ? "随机 API" : item.origin === "folder" ? `文件夹 · ${item.fileCount ?? 0} 个媒体` : item.origin === "remote" ? "网络媒体" : "本地媒体"}${item.origin === "folder" ? "" : ` · ${formatBytes(item.byteSize)}`}` : "从左侧媒体库选择"}</small></span>
      </div>
    </section>
  );
}

export default function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [tab, setTab] = useState<InspectorTab>("display");
  const [route, setRoute] = useState<PreviewRoute>("home");
  const [query, setQuery] = useState("");
  const [remoteOpen, setRemoteOpen] = useState(false);
  const [remoteUrl, setRemoteUrl] = useState("");
  const [remoteDynamic, setRemoteDynamic] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [operationPending, setOperationPending] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const updateTimer = useRef<number | null>(null);
  const busyTimer = useRef<number | null>(null);
  const operationPendingRef = useRef(false);

  useEffect(() => {
    bridge.getSnapshot().then(setSnapshot).catch((error) => setNotice(error.message));
    return bridge.onSnapshot(setSnapshot);
  }, []);

  useEffect(() => () => {
    if (updateTimer.current !== null) window.clearTimeout(updateTimer.current);
    if (busyTimer.current !== null) window.clearTimeout(busyTimer.current);
  }, []);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 4200);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const run = useCallback(async (label: string, operation: () => Promise<unknown>) => {
    if (operationPendingRef.current) return;
    operationPendingRef.current = true;
    setOperationPending(true);
    busyTimer.current = window.setTimeout(() => {
      setBusy(label);
      busyTimer.current = null;
    }, BUSY_INDICATOR_DELAY_MS);
    try { await operation(); }
    catch (error) { setNotice((error as Error).message); }
    finally {
      if (busyTimer.current !== null) window.clearTimeout(busyTimer.current);
      busyTimer.current = null;
      setBusy(null);
      setOperationPending(false);
      operationPendingRef.current = false;
    }
  }, []);

  const patchSettings = useCallback((patch: SettingsPatch) => {
    setSnapshot((current) => current ? {
      ...current,
      settings: {
        ...current.settings,
        ...patch,
        display: { ...current.settings.display, ...patch.display },
        slideshow: { ...current.settings.slideshow, ...patch.slideshow },
        behavior: { ...current.settings.behavior, ...patch.behavior },
      },
    } : current);
    if (updateTimer.current) window.clearTimeout(updateTimer.current);
    updateTimer.current = window.setTimeout(() => {
      bridge.updateSettings(patch).then(setSnapshot).catch((error) => setNotice(error.message));
    }, 120);
  }, []);

  const importResult = useCallback(async (operation: () => Promise<{ added: MediaItem[]; skipped: Array<{ reason: string }> }>) => {
    const result = await operation();
    if (result.added.length || result.skipped.length) {
      const folder = result.added.find((item) => item.origin === "folder");
      setNotice(result.added.length
        ? folder
          ? `已添加文件夹源「${folder.name}」，不会复制文件`
          : `已导入 ${result.added.length} 个媒体${result.skipped.length ? `，跳过 ${result.skipped.length} 个` : ""}`
        : result.skipped[0]?.reason ?? "没有可导入的媒体");
    }
    setSnapshot(await bridge.getSnapshot());
  }, []);

  const active = snapshot?.library.find((item) => item.id === snapshot.settings.activeMediaId) ?? null;
  const filtered = useMemo(() => snapshot?.library.filter((item) =>
    item.name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())) ?? [], [snapshot?.library, query]);

  if (!snapshot) return <div className="app-loading"><RefreshCw className="spin" size={24} />正在载入背景管理器</div>;

  const display = snapshot.settings.display;
  const phase = snapshot.runtime.phase;
  const phaseTone = phase === "active" ? "active" : phase === "error" ? "error" : phase === "paused" ? "paused" : "idle";

  const togglePlaylist = (id: string) => {
    const ids = snapshot.settings.playlistIds;
    patchSettings({ playlistIds: ids.includes(id) ? ids.filter((candidate) => candidate !== id) : [...ids, id] });
  };

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand"><span className="brand-mark"><img src={brandIcon} alt="" width={34} height={34} /></span><span><strong>Multica Background Studio</strong><small>背景管理器</small></span></div>
        <div className={`runtime-status status-${phaseTone}`}><i /><span>{snapshot.runtime.message}</span>{snapshot.runtime.multicaVersion && <small>{snapshot.runtime.multicaVersion}</small>}</div>
        <div className="top-actions">
          <ActionButton icon={<Play size={16} />} tone="primary" disabled={!active || operationPending} onClick={() => run("apply", () => bridge.apply())}>应用</ActionButton>
          <ActionButton icon={<CirclePause size={16} />} disabled={phase !== "active" || operationPending} onClick={() => run("pause", () => bridge.pause())}>暂停</ActionButton>
          <ActionButton icon={<RefreshCw size={16} />} disabled={operationPending} onClick={() => run("restore", () => bridge.restore())}>恢复</ActionButton>
        </div>
      </header>

      <div className="workspace">
        <aside className="library-panel">
          <div className="panel-heading"><div><h2>媒体库</h2><span>{snapshot.library.length}</span></div><ChevronDown size={16} /></div>
          <div className="import-actions">
            <IconButton icon={<Upload size={17} />} label="导入文件" onClick={() => run("files", () => importResult(() => bridge.chooseMediaFiles()))} />
            <IconButton icon={<FolderOpen size={17} />} label="添加文件夹" onClick={() => run("folder", () => importResult(() => bridge.chooseMediaFolder()))} />
            <IconButton icon={<Link size={17} />} label="添加网络媒体" onClick={() => setRemoteOpen(true)} />
          </div>
          <label className="search-box"><Search size={15} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索媒体" /></label>
          <div className="media-list">
            {filtered.length ? filtered.map((item) => (
              <MediaThumb
                key={item.id}
                item={item}
                active={item.id === snapshot.settings.activeMediaId}
                inPlaylist={snapshot.settings.playlistIds.includes(item.id)}
                onActivate={() => run("activate", () => bridge.setActiveMedia(item.id))}
                onTogglePlaylist={() => togglePlaylist(item.id)}
                onRefresh={() => run("refresh", async () => setSnapshot(await bridge.refreshMedia(item.id)))}
                onRemove={() => run("remove", () => bridge.removeMedia(item.id))}
              />
            )) : (
              <div className="library-empty"><ImageIcon size={28} /><strong>{query ? "没有匹配项" : "媒体库为空"}</strong><span>{query ? "换一个关键词" : "导入文件或添加网络地址"}</span></div>
            )}
          </div>
        </aside>

        <Preview
          item={active}
          display={display}
          route={route}
          onRouteChange={setRoute}
          onPositionChange={(positionX, positionY) => patchSettings({ display: { positionX, positionY } })}
        />

        <aside className="inspector-panel">
          <nav className="inspector-tabs">
            {([
              ["display", <SlidersHorizontal size={16} />, "画面"],
              ["pages", <MonitorUp size={16} />, "页面"],
              ["slideshow", <Shuffle size={16} />, "轮播"],
              ["settings", <Settings size={16} />, "设置"],
            ] as const).map(([id, icon, label]) => (
              <button key={id} className={tab === id ? "is-active" : ""} onClick={() => setTab(id)} title={label}>{icon}<span>{label}</span></button>
            ))}
          </nav>

          <div className="inspector-content">
            {tab === "display" && <>
              <section className="control-section">
                <h3>填充</h3>
                <div className="segmented fit-segmented">
                  {(Object.keys(fitLabels) as FitMode[]).map((fit) => <button key={fit} className={display.fit === fit ? "is-active" : ""} onClick={() => patchSettings({ display: { fit } })}>{fitLabels[fit]}</button>)}
                </div>
              </section>
              <section className="control-section">
                <RangeControl label="背景图不透明度" value={display.opacity} min={0} max={1} step={0.01} onChange={(opacity) => patchSettings({ display: { opacity } })} />
                <RangeControl label="模糊" value={display.blur} min={0} max={40} step={1} suffix=" px" onChange={(blur) => patchSettings({ display: { blur } })} />
                <RangeControl label="缩放" value={display.scale} min={1} max={1.3} step={0.01} onChange={(scale) => patchSettings({ display: { scale } })} />
                <RangeControl label="水平位置" value={display.positionX} min={0} max={100} step={1} suffix="%" onChange={(positionX) => patchSettings({ display: { positionX } })} />
                <RangeControl label="垂直位置" value={display.positionY} min={0} max={100} step={1} suffix="%" onChange={(positionY) => patchSettings({ display: { positionY } })} />
              </section>
              <section className="control-section">
                <h3>底色</h3>
                <p className="section-hint">铺在背景图上面、界面下面的整页底色，用来压对比或统一色调。</p>
                <label className="color-control"><span>颜色</span><span className="color-input"><input type="color" value={display.overlayColor} onChange={(event) => patchSettings({ display: { overlayColor: event.target.value } })} /><code>{display.overlayColor}</code></span></label>
                <RangeControl label="强度" value={display.overlayOpacity} min={0} max={0.9} step={0.01} onChange={(overlayOpacity) => patchSettings({ display: { overlayOpacity } })} />
              </section>
              {active?.kind === "video" && <section className="control-section">
                <h3>视频</h3>
                <Toggle label="静音播放" checked={display.videoMuted} onChange={(videoMuted) => patchSettings({ display: { videoMuted } })} />
                <RangeControl label="播放速度" value={display.videoPlaybackRate} min={0.25} max={2} step={0.25} onChange={(videoPlaybackRate) => patchSettings({ display: { videoPlaybackRate } })} />
              </section>}
            </>}

            {tab === "pages" && <>
              <section className="control-section">
                <h3>页面</h3>
                <Toggle label="显示背景" checked={display.enabledOnHome} onChange={(enabledOnHome) => patchSettings({ display: { enabledOnHome } })} />
                <RangeControl label="显示强度" value={display.homeIntensity} min={0} max={1} step={0.01} onChange={(homeIntensity) => patchSettings({ display: { homeIntensity } })} />
              </section>
              <section className="control-section">
                <h3>空白页</h3>
                <p className="section-hint">次要路由强度。强度为 0 时可关掉该路由背景，只保留主页面。</p>
                <Toggle label="显示背景" checked={display.enabledOnTasks} onChange={(enabledOnTasks) => patchSettings({ display: { enabledOnTasks } })} />
                <RangeControl label="显示强度" value={display.taskIntensity} min={0} max={1} step={0.01} onChange={(taskIntensity) => patchSettings({ display: { taskIntensity } })} />
              </section>
              <section className="control-section">
                <h3>界面层</h3>
                <p className="section-hint">给壳层加一点雾。0 为全透，露出整张背景。</p>
                <RangeControl label="左侧边栏" value={display.sidebarOpacity} min={0} max={1} step={0.01} onChange={(sidebarOpacity) => patchSettings({ display: { sidebarOpacity } })} />
                <RangeControl label="顶栏与页面" value={display.surfaceOpacity} min={0} max={1} step={0.01} onChange={(surfaceOpacity) => patchSettings({ display: { surfaceOpacity } })} />
                <RangeControl label="弹出菜单" value={display.menuOpacity} min={0} max={1} step={0.01} onChange={(menuOpacity) => patchSettings({ display: { menuOpacity } })} />
              </section>
            </>}

            {tab === "slideshow" && <>
              <section className="control-section">
                <Toggle label="启用自动轮播" checked={snapshot.settings.slideshow.enabled} onChange={(enabled) => patchSettings({ slideshow: { enabled } })} />
                <label className="select-control"><span>切换顺序</span><select value={snapshot.settings.slideshow.order} onChange={(event) => patchSettings({ slideshow: { order: event.target.value as "sequential" | "random" } })}><option value="sequential">顺序播放</option><option value="random">随机播放</option></select></label>
                <RangeControl label="间隔" value={snapshot.settings.slideshow.intervalSeconds} min={10} max={3600} step={10} suffix=" 秒" onChange={(intervalSeconds) => patchSettings({ slideshow: { intervalSeconds } })} />
              </section>
              <section className="control-section playlist-section">
                <h3>播放列表 <span>{snapshot.settings.playlistIds.length}</span></h3>
                {snapshot.settings.playlistIds.map((id, index) => {
                  const item = snapshot.library.find((candidate) => candidate.id === id);
                  return item ? <div className="playlist-row" key={id}><span>{index + 1}</span><strong>{item.name}</strong><IconButton icon={<X size={14} />} label="从轮播移除" onClick={() => togglePlaylist(id)} /></div> : null;
                })}
              </section>
            </>}

            {tab === "settings" && <>
              <section className="control-section">
                <h3>窗口</h3>
                <Toggle label="关闭时最小化到托盘" checked={snapshot.settings.behavior.closeToTray} onChange={(closeToTray) => patchSettings({ behavior: { closeToTray } })} />
                <Toggle label="随 Windows 启动" checked={snapshot.settings.behavior.autoStartWithWindows} onChange={(autoStartWithWindows) => patchSettings({ behavior: { autoStartWithWindows } })} />
                <Toggle label="启动时隐藏窗口" checked={snapshot.settings.behavior.startMinimized} onChange={(startMinimized) => patchSettings({ behavior: { startMinimized } })} />
              </section>
              <section className="control-section">
                <h3>数据</h3>
                <button className="wide-command" onClick={() => bridge.openDataDirectory()}><FolderOpen size={16} /><span>打开数据目录</span><ExternalLink size={14} /></button>
                <code className="data-path">{snapshot.dataDirectory}</code>
              </section>
              <section className="control-section restore-section">
                <h3>官方外观</h3>
                <ActionButton icon={<RefreshCw size={16} />} tone="danger" onClick={() => run("restore", () => bridge.restore())}>移除背景并重启 Multica</ActionButton>
              </section>
            </>}
          </div>
        </aside>
      </div>

      {remoteOpen && <div className="dialog-backdrop" onMouseDown={() => setRemoteOpen(false)}>
        <form className="dialog" onSubmit={(event) => {
          event.preventDefault();
          if (!remoteUrl.trim()) return;
          setRemoteOpen(false);
          void run("remote", () => importResult(() => bridge.addRemoteMedia({ url: remoteUrl.trim(), dynamic: remoteDynamic })))
            .then(() => { setRemoteUrl(""); setRemoteDynamic(false); });
        }} onMouseDown={(event) => event.stopPropagation()}>
          <div className="dialog-title"><span><Link size={18} />添加网络媒体</span><IconButton icon={<X size={16} />} label="关闭" onClick={() => setRemoteOpen(false)} /></div>
          <label><span>图片或视频地址</span><input autoFocus type="url" value={remoteUrl} onChange={(event) => setRemoteUrl(event.target.value)} placeholder="https://example.com/background.jpg" /></label>
          <Toggle label="随机图片 API（轮播时每次重新请求）" checked={remoteDynamic} onChange={setRemoteDynamic} />
          <div className="dialog-actions"><button type="button" onClick={() => setRemoteOpen(false)}>取消</button><button type="submit" className="primary" disabled={!remoteUrl.trim()}>{remoteDynamic ? "添加随机源" : "下载并添加"}</button></div>
        </form>
      </div>}

      {busy && <div className="busy-indicator"><RefreshCw className="spin" size={15} />正在处理</div>}
      {notice && <div className="notice" role="status"><span>{notice}</span><button onClick={() => setNotice(null)} aria-label="关闭通知"><X size={15} /></button></div>}
    </main>
  );
}
