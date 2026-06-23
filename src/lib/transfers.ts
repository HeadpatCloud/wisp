import { cancelFtp } from '@/lib/ftp'
import { cancelTransfer as cancelSftp } from '@/lib/sftp'

// Transfers from SFTP and FTP panels share one global TransfersBar, and a transfer id is
// unique across both, so we fire both cancels - the backend that owns the id acts, the
// other no-ops on the unknown id.
export async function cancelTransfer(transferId: string): Promise<void> {
  await Promise.allSettled([cancelSftp(transferId), cancelFtp(transferId)])
}
