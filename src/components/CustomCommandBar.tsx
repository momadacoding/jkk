import { useCallback, useState } from 'react'
import { planCustomCommand, type CommandContext } from '../engine/customCommands'
import { toErrorMessage } from '../engine/errors'
import { useCustomCommands } from '../store/customCommandsStore'
import { ContextMenu, useContextMenu } from './ContextMenu'

/**
 * 工具栏右侧的自定义命令组。
 *
 * 命令是启动外部程序，毫秒级返回，所以不做 busy 态——那是留给 fetch/push 的。
 */
export function CustomCommandBar({ context }: { context: CommandContext }) {
  const { commands, error, reload, run, openConfig } = useCustomCommands()
  const { menu, open, close } = useContextMenu()
  const [failure, setFailure] = useState<string | null>(null)

  const launch = useCallback(
    async (id: string) => {
      const cmd = commands.find((c) => c.id === id)
      if (!cmd) return
      const plan = planCustomCommand(cmd, context)
      if (!plan) return
      try {
        setFailure(null)
        await run(plan)
      } catch (err) {
        setFailure(toErrorMessage(err))
      }
    },
    [commands, context, run],
  )

  const openMenu = (e: React.MouseEvent) => {
    open(e, [
      { label: 'Edit commands…', onSelect: () => void openConfig() },
      { label: 'Reload', onSelect: () => void reload() },
    ])
  }

  return (
    <>
      {commands.map((cmd, i) => (
        <button
          // id 允许重复（后端不因此报错），所以 key 不能只用 id
          key={`${cmd.id}-${i}`}
          className="toolbar-button"
          disabled={context.worktree === null}
          onClick={() => void launch(cmd.id)}
          data-command={cmd.id}
          title={`${cmd.program} ${cmd.args.join(' ')}`}
        >
          {cmd.label}
        </button>
      ))}
      <button
        className="toolbar-button"
        // 编辑配置不需要先打开仓库，所以这个按钮不跟随 worktree 禁用
        onClick={openMenu}
        title={error ?? failure ?? '自定义命令'}
        data-command="settings"
      >
        {error || failure ? '⚠' : '⚙'}
      </button>
      <ContextMenu menu={menu} onClose={close} />
    </>
  )
}
