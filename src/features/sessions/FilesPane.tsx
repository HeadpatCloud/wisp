import { useCallback } from 'react'
import { FtpConnectionView } from '@/features/ftp/FtpConnectionView'
import { S3ConnectionView } from '@/features/s3/S3ConnectionView'
import {
  type FtpPane,
  type S3Pane,
  type SessionStatus,
  type SftpPane,
  useSessionStore,
} from '@/stores/sessionStore'
import { PaneFrame, PaneHeader } from './PaneHeader'
import { SftpConnectionView } from './SftpConnectionView'

function FilesView({ pane, active }: { pane: SftpPane | FtpPane | S3Pane; active: boolean }) {
  const setStatus = useSessionStore((s) => s.setStatus)
  const onStatus = useCallback(
    (status: SessionStatus) => setStatus(pane.id, status),
    [setStatus, pane.id],
  )
  switch (pane.kind) {
    case 'sftp':
      return (
        <SftpConnectionView
          name={pane.title}
          onStatus={onStatus}
          profileId={pane.sshConnectionId}
          sftpProfileId={pane.connectionId}
          adhoc={pane.adhoc}
          viaSession={pane.viaSession}
          active={active}
        />
      )
    case 'ftp':
      return (
        <FtpConnectionView
          name={pane.title}
          onStatus={onStatus}
          connectionId={pane.connectionId}
          host={pane.host}
          port={pane.port}
          username={pane.username}
          secretId={pane.secretId}
          secure={pane.secure}
          allowInvalidCert={pane.allowInvalidCert}
          ignoreHostname={pane.ignoreHostname}
          active={active}
        />
      )
    case 's3':
      return (
        <S3ConnectionView
          name={pane.title}
          onStatus={onStatus}
          profileId={pane.connectionId}
          bucket={pane.bucket}
          active={active}
        />
      )
  }
}

export function FilesPane({
  tabId,
  pane,
  active,
}: {
  tabId: string
  pane: SftpPane | FtpPane | S3Pane
  active: boolean
}) {
  return (
    <PaneFrame tabId={tabId} paneId={pane.id}>
      <PaneHeader tabId={tabId} paneId={pane.id} />
      <div className="min-h-0 flex-1">
        <FilesView pane={pane} active={active} />
      </div>
    </PaneFrame>
  )
}
