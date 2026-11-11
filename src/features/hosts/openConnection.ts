import type { Connection, Host } from '@/bindings'
import { connectionLabel, effectiveAddress, loginUser } from '@/lib/hosts'
import { useHostStore } from '@/stores/hostStore'
import { type SplitDirection, useSessionStore } from '@/stores/sessionStore'

// Tabs hold the connection id; the backend reads the connection again on every connect, so an
// edit is picked up without reopening the tab. Remote desktops never split: they always get a tab.
export function openConnection(host: Host, c: Connection, split?: SplitDirection): void {
  const opts = { split }
  const st = useSessionStore.getState()
  const k = c.kind
  switch (k.type) {
    case 'ssh':
      st.openSsh(c.id, host.name, opts)
      return
    case 'sftp':
      st.openSftpProfile(c.id, host.name, opts)
      return
    case 's3':
      st.openS3(c.id, k.bucket, host.name, opts)
      return
    case 'vnc':
      st.openVnc({ connectionId: c.id, title: host.name })
      return
    case 'rdp':
      st.openRdp({ connectionId: c.id, title: host.name })
      return
    case 'local': {
      const { logins, shells } = useHostStore.getState()
      st.openLocalShell(
        k.program,
        connectionLabel(host, c, logins, shells),
        k.startFolder,
        c.id,
        opts,
      )
      return
    }
    case 'ftp': {
      const username = loginUser(k.login, useHostStore.getState().logins) ?? ''
      st.openFtpSaved(
        c.id,
        host.name,
        { host: effectiveAddress(host, c), port: k.port, username, secure: k.secure },
        opts,
      )
      return
    }
  }
}

export function openFiles(host: Host, sshConnectionId: string, split?: SplitDirection): void {
  useSessionStore.getState().openSftp(sshConnectionId, host.name, { split })
}
