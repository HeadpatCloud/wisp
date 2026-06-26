import { cancelFtp } from '@/lib/ftp'
import { cancelS3 } from '@/lib/s3'
import { cancelTransfer as cancelSftp } from '@/lib/sftp'

// Transfers from SFTP, FTP and S3 panels share one global TransfersBar, and a transfer id is
// unique across all, so we fire every cancel - the backend that owns the id acts, the others
// no-op on the unknown id.
export async function cancelTransfer(transferId: string): Promise<void> {
  await Promise.allSettled([cancelSftp(transferId), cancelFtp(transferId), cancelS3(transferId)])
}
