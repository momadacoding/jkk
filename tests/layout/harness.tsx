/**
 * 布局测试的复现页：用真实组件、真实 styles.css、真实的文案函数把每个弹框撑到超长。
 * 只由 dev server 提供，不进 `vite build` 的产物。
 */
import { useEffect, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import '@/styles.css'
import { useConfirm, type ConfirmRequest } from '@/components/ConfirmDialog'
import { DeleteBranchDialog } from '@/components/DeleteBranchDialog'
import { DeleteWorktreeDialog } from '@/components/DeleteWorktreeDialog'
import { useDeliverDialog } from '@/components/DeliverDialog'
import { useFollowReportDialog } from '@/components/FollowReportDialog'
import { usePrompt } from '@/components/PromptDialog'
import { SettingsDialog } from '@/components/SettingsDialog'
import SvnInitWizard from '@/components/SvnInitWizard'
import { TabBar } from '@/components/TabBar'
import { TsvnHooksDialog } from '@/components/TsvnHooksDialog'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import { describeDiscard } from '@/engine/discardConfirm'
import type { FollowReport } from '@/engine/svnFollow'
import { describeHealth } from '@/engine/svnInit'
import { describeSyncInPreview } from '@/engine/svnSyncIn'
import { describeDeliverPreview } from '@/engine/svnSyncOut'
import type { HookEntry } from '@/engine/tsvnHooks'

const paths = (n: number) =>
  Array.from({ length: n }, (_, i) => `Assets/Scripts/Gameplay/Battle/Skill/SkillEffect${i}.cs`)
const LONG_ERROR = paths(200).join('\n')
const noop = () => {}

// `invoke` 走这个全局。删除分支的框一打开就读分支信息，这里让它失败，错误文案撑满框
;(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
  invoke: async () => {
    throw new Error(LONG_ERROR)
  },
}

function OpenOnMount({ open, children }: { open: () => void; children: ReactNode }) {
  useEffect(() => {
    open()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  return <>{children}</>
}

function Confirm({ request }: { request: ConfirmRequest }) {
  const { confirm, dialog } = useConfirm()
  return <OpenOnMount open={() => void confirm(request)}>{dialog}</OpenOnMount>
}

function Prompt() {
  const { prompt, dialog } = usePrompt()
  return (
    <OpenOnMount open={() => void prompt({ title: '长说明', message: LONG_ERROR })}>
      {dialog}
    </OpenOnMount>
  )
}

function Deliver({ fail = false }: { fail?: boolean }) {
  const { deliver, dialog } = useDeliverDialog()
  const model = describeDeliverPreview({
    origBranch: 'work',
    origHead: 'h'.repeat(40),
    baselineOid: 'b'.repeat(40),
    baselineRev: 1,
    remoteRev: 1,
    deliverOid: 'd'.repeat(40),
    deliverTree: 't'.repeat(40),
    changes: paths(300).map((path) => ({ status: 'M' as const, path })),
    localOnly: paths(100).map((subject, i) => ({ oid: String(i).padEnd(40, '0'), subject })),
    // 失败场景不带 outsideChanges：带了要先勾确认才点得动交付
    outsideChanges: fail ? [] : paths(100),
  })
  return (
    <OpenOnMount
      open={() =>
        void deliver({
          model,
          initialMessage: '修复跳跃',
          onMessageChange: noop,
          action: async () => {
            if (fail) throw new Error('[svn_commit_failed] 服务器拒绝了这次提交')
          },
        })
      }
    >
      {dialog}
    </OpenOnMount>
  )
}

function Follow() {
  const { showFollowReport, dialog } = useFollowReportDialog()
  const report: FollowReport = {
    newFloor: 120,
    verified: false,
    changes: paths(300).map((path) => ({ path, rev: 120, author: 'someone' })),
    removed: paths(100),
    dropped: [],
    rewritten: [],
    snapshot: [['f'.repeat(40), paths(100)]],
    drift: paths(100),
    stage: { decisions: {}, uncertain: paths(100), lost: paths(100), discarded: [] },
    movedBranches: [],
    behindBranches: [],
    leaks: [],
    pinnedRef: null,
    evidence: 'heuristic',
    evidenceCutoffMs: null,
  }
  return (
    <OpenOnMount
      open={() =>
        void showFollowReport({
          report,
          readUser: async () => ({ configured: null, detected: null }),
          saveUser: async () => {},
        })
      }
    >
      {dialog}
    </OpenOnMount>
  )
}

function hookEntries(n: number): HookEntry[] {
  return paths(n).map((wcPath) => ({
    kind: 'post_commit',
    wcPath,
    command: `"C:\\Program Files\\jkk\\jkk.exe" --hook post_commit ${wcPath}`,
    wait: 'true',
    show: 'false',
    enforce: 'false',
  }))
}

const SCENES: Record<string, () => ReactNode> = {
  'confirm-long-warning': () => {
    // RepoTab.handleCheckBaseline 的形状
    const d = describeHealth({ status: 'lying', files: paths(300) })
    return (
      <Confirm
        request={{
          title: d.title,
          message: '下面这些文件，基线记的内容与服务器上的不一致：',
          warning: d.detail,
          confirmLabel: '重建基线',
        }}
      />
    )
  },
  'confirm-long-message': () => (
    <Confirm
      request={{ title: '长正文', message: LONG_ERROR, confirmLabel: '知道了', notice: 'attention' }}
    />
  ),
  'confirm-long-error': () => (
    <Confirm
      request={{
        ...describeSyncInPreview({
          origBranch: 'work',
          origHead: 'a'.repeat(40),
          baselineOid: 'b'.repeat(40),
          baselineRev: 100,
          targetRev: 120,
          catchUpRev: 110,
          outsideChanges: paths(20),
        }),
        action: async () => {
          throw new Error(LONG_ERROR)
        },
      }}
    />
  ),
  // Ctrl+A 全选后丢弃：两段都很长
  'confirm-discard-long': () => (
    <Confirm request={describeDiscard(paths(600), paths(300), false)} />
  ),
  'prompt-long-message': () => <Prompt />,
  'delete-branch-long-error': () => (
    <DeleteBranchDialog
      repoPath="C:\\repo"
      name="feat/x"
      capabilities={ALL_CAPABILITIES}
      mergedPullRequests={[]}
      onCancel={noop}
      onConfirm={async () => {}}
    />
  ),
  'delete-worktree-long-error': () => (
    <DeleteWorktreeDialog
      worktree={{ name: 'wt', path: 'C:/r/.worktrees/wt', head: 'h', branch: 'feat/x', isMain: false }}
      onCancel={noop}
      onConfirm={async () => {
        throw new Error(LONG_ERROR)
      }}
    />
  ),
  // 向导不是弹框，住在标签页里：照 App.tsx 套上标签栏和 .tab-panel，标签栏占掉的高度也要算进去
  'wizard-long-error': () => (
    <div className="app">
      <TabBar
        tabs={[{ id: 't', path: 'C:\\repo' }]}
        activeId="t"
        onActivate={noop}
        onClose={noop}
        onCloseAll={noop}
        onCloseOthers={noop}
        onOpenRepo={noop}
      />
      <div className="tab-panel" style={{ display: 'flex' }}>
      <div className="repo-tab">
        <SvnInitWizard
          repoPath="C:\\repo"
          onCancel={noop}
          onFinish={noop}
          onPreflight={async () => {
            throw new Error(LONG_ERROR)
          }}
          onInit={async () => {
            throw new Error(LONG_ERROR)
          }}
        />
      </div>
      </div>
    </div>
  ),
  'deliver-long': () => <Deliver />,
  'deliver-short-error': () => <Deliver fail />,
  'follow-report-long': () => <Follow />,
  'tsvn-hooks-long': () => (
    <TsvnHooksDialog
      status={{
        supported: true,
        health: { state: 'notRegistered' },
        planned: hookEntries(60),
        conflicts: hookEntries(60),
        exe: 'C:\\Program Files\\jkk\\jkk.exe',
      }}
      error={LONG_ERROR}
      onRegister={noop}
      onUnregister={noop}
      onClose={noop}
    />
  ),
  'settings-long-error': () => (
    <SettingsDialog
      initial={{
        closeToTray: true,
        trayAvailable: true,
        showShortcut: 'Ctrl+Alt+Shift+Super+F12',
        shortcutError: LONG_ERROR,
        defaultShortcut: 'Ctrl+Alt+J',
        svnAutoFollow: true,
      }}
      onSave={async () => {
        throw new Error(LONG_ERROR)
      }}
      onClose={noop}
    />
  ),
  'settings-load-failed': () => (
    <SettingsDialog initial={null} loadError={LONG_ERROR} onSave={async () => {}} onClose={noop} />
  ),
}

const id = new URLSearchParams(location.search).get('scenario') ?? ''
const scene = SCENES[id]
if (!scene) throw new Error(`复现页没有这个场景：${id}`)
createRoot(document.getElementById('root')!).render(scene())
