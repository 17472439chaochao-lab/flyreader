/** 确认对话框关闭时的结果：确认 / 取消 / 额外操作 */
export type ConfirmOutcome = 'confirm' | 'cancel' | 'extra'

/** 确认对话框请求参数（含 Promise resolve） */
export type ConfirmRequest = {
  title: string
  message: string
  confirmText?: string
  cancelText?: string
  /** 可选第三操作按钮文案（例如「换源」） */
  extraText?: string
  danger?: boolean
  resolve: (result: ConfirmOutcome) => void
}

/** 确认对话框组件属性 */
export type ConfirmDialogProps = {
  request: ConfirmRequest
  onClose: (result: ConfirmOutcome) => void
}

/**
 * 通用确认对话框：支持确认、取消，以及可选的第三操作按钮。
 * 点击遮罩视为取消；危险操作时主按钮使用 danger 样式。
 */
export function ConfirmDialog({ request, onClose }: ConfirmDialogProps) {
  return (
    <div className="modal-backdrop confirm-backdrop" onClick={() => onClose('cancel')}>
      <div
        className={`confirm-dialog ${request.danger ? 'is-danger' : ''}`}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        aria-describedby="confirm-desc"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="confirm-title">{request.title}</h3>
        <p id="confirm-desc">{request.message}</p>
        <div className="confirm-actions">
          <button type="button" className="confirm-btn ghost" onClick={() => onClose('cancel')}>
            {request.cancelText || '取消'}
          </button>
          {request.extraText ? (
            <button type="button" className="confirm-btn ghost" onClick={() => onClose('extra')}>
              {request.extraText}
            </button>
          ) : null}
          <button
            type="button"
            className={`confirm-btn primary ${request.danger ? 'danger' : ''}`}
            autoFocus
            onClick={() => onClose('confirm')}
          >
            {request.confirmText || '确定'}
          </button>
        </div>
      </div>
    </div>
  )
}
