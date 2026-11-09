/**
 * 布局测试的场景清单。与 `harness.tsx` 按 id 对应。
 *
 * 单独成文件、不依赖 React：`tests/vitest/dialogGuards.test.ts` 要读它，核对每个弹框组件都有
 * 场景。新增弹框而没加场景时 `pnpm test` 会失败，布局测试才不会悄悄漏掉它。
 */
export interface LayoutScenario {
  id: string
  /** 被测的弹框组件，与 `src/components/` 下的文件名一致。 */
  component: string
  /** 打开后点主按钮：错误要等 action 失败才出现。 */
  clickPrimary?: boolean
  /**
   * 打开后先点这个元素，再等 `thenShows` 这段文字出现才开始量：有的说明要用户操作之后才出现
   * （勾上「顺便删远程」→ 探测答「远程已经没有」）。
   */
  clickFirst?: { selector: string; thenShows: string }
}

export const SCENARIOS: LayoutScenario[] = [
  { id: 'confirm-long-warning', component: 'ConfirmDialog' },
  { id: 'confirm-long-message', component: 'ConfirmDialog' },
  { id: 'confirm-long-error', component: 'ConfirmDialog', clickPrimary: true },
  { id: 'confirm-discard-long', component: 'ConfirmDialog' },
  { id: 'prompt-long-message', component: 'PromptDialog' },
  { id: 'delete-branch-long-error', component: 'DeleteBranchDialog' },
  {
    id: 'delete-branch-remote-gone',
    component: 'DeleteBranchDialog',
    clickFirst: { selector: '.dlg-check input >> nth=0', thenShows: '远程上已经没有这条分支了' },
    clickPrimary: true,
  },
  { id: 'delete-worktree-long-error', component: 'DeleteWorktreeDialog', clickPrimary: true },
  { id: 'deliver-long', component: 'DeliverDialog' },
  // 错误在内容末尾、只有一行：从顶部点交付，它最容易出在视野外或藏在按钮条下面
  { id: 'deliver-short-error', component: 'DeliverDialog', clickPrimary: true },
  { id: 'follow-report-long', component: 'FollowReportDialog' },
  { id: 'tsvn-hooks-long', component: 'TsvnHooksDialog' },
  // 已保存的快捷键注册失败的说明 + 保存失败的错误同时在框里
  { id: 'settings-long-error', component: 'SettingsDialog', clickPrimary: true },
  { id: 'settings-load-failed', component: 'SettingsDialog' },
]
