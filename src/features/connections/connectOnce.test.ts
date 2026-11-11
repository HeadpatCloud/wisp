import { beforeEach, expect, test } from 'vitest'
import { parseAddress } from '@/lib/address'
import { useHostsUiStore } from '@/stores/hostsUiStore'
import { connectOnce, useConnectOnce } from './connectOnce'

beforeEach(() => {
  useConnectOnce.setState({ request: null })
  useHostsUiStore.setState(useHostsUiStore.getInitialState())
})

test.each([
  ['admin@nas.local', 'ssh'],
  ['sftp://nas.local', 'sftp'],
  ['ftps://nas.local', 'ftps'],
  ['rdp://win-dev', 'rdp'],
  ['vnc://media-box', 'vnc'],
])('a parsed address connects once through its type’s dialog: %s', (text, type) => {
  connectOnce(parseAddress(text) as NonNullable<ReturnType<typeof parseAddress>>)
  expect(useConnectOnce.getState().request?.type).toBe(type)
  expect(useHostsUiStore.getState().page).toBe('hosts')
})

test('an S3 address opens New connection filled in', () => {
  connectOnce(
    parseAddress('s3://minio.lan:9000/logs') as NonNullable<ReturnType<typeof parseAddress>>,
  )
  expect(useConnectOnce.getState().request).toBeNull()
  const ui = useHostsUiStore.getState()
  expect(ui.page).toBe('new')
  expect(ui.newConnection).toEqual({ type: 's3', address: 'minio.lan', port: 9000, bucket: 'logs' })
})
