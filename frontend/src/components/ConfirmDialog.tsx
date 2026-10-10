import { Button, Group, Modal, Text } from '@mantine/core'

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
 * 点击遮罩与 Esc 均视为取消；危险操作时主按钮使用 danger 配色。
 */
export function ConfirmDialog({ request, onClose }: ConfirmDialogProps) {
  return (
    <Modal
      opened
      centered
      size="sm"
      withCloseButton={false}
      title={request.title}
      onClose={() => onClose('cancel')}
    >
      <Text size="sm" c="dimmed" style={{ whiteSpace: 'pre-wrap' }}>
        {request.message}
      </Text>
      <Group justify="flex-end" mt="md">
        <Button variant="default" size="sm" onClick={() => onClose('cancel')}>
          {request.cancelText || '取消'}
        </Button>
        {request.extraText ? (
          <Button variant="default" size="sm" onClick={() => onClose('extra')}>
            {request.extraText}
          </Button>
        ) : null}
        <Button
          size="sm"
          color={request.danger ? 'danger' : 'accent'}
          autoFocus
          onClick={() => onClose('confirm')}
        >
          {request.confirmText || '确定'}
        </Button>
      </Group>
    </Modal>
  )
}
