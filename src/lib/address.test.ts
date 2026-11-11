import { describe, expect, test } from 'vitest'
import { addressKey, describeAddress, type ParsedAddress, parseAddress } from './address'

const p = (
  type: ParsedAddress['type'],
  host: string,
  port: number | null,
  user: string | null = null,
  bucket: string | null = null,
): ParsedAddress => ({ type, host, port, user, bucket })

describe('parse_table', () => {
  test.each<[string, ParsedAddress | null]>([
    ['nas.local', p('ssh', 'nas.local', 22)],
    ['  admin@nas.local  ', p('ssh', 'nas.local', 22, 'admin')],
    ['nas.local:2222', p('ssh', 'nas.local', 2222)],
    ['admin@nas.local:2222', p('ssh', 'nas.local', 2222, 'admin')],
    ['[2001:db8::1]', p('ssh', '2001:db8::1', 22)],
    ['[2001:db8::1]:2200', p('ssh', '2001:db8::1', 2200)],
    ['fe80::1', p('ssh', 'fe80::1', 22)],
    ['ssh://root@10.0.0.5:22', p('ssh', '10.0.0.5', 22, 'root')],
    ['sftp://nas.local', p('sftp', 'nas.local', 22)],
    ['ftp://media@nas.local', p('ftp', 'nas.local', 21, 'media')],
    ['FTPS://nas.local:990', p('ftps', 'nas.local', 990)],
    ['rdp://admin@win-dev.corp', p('rdp', 'win-dev.corp', 3389, 'admin')],
    ['vnc://192.168.1.31:5901', p('vnc', '192.168.1.31', 5901)],
    [
      's3://s3.eu-central-1.amazonaws.com/wisp-backups',
      p('s3', 's3.eu-central-1.amazonaws.com', null, null, 'wisp-backups'),
    ],
    ['s3://minio.lan:9000', p('s3', 'minio.lan', 9000)],
    ['user@domain@host', p('ssh', 'host', 22, 'user@domain')],
    ['ftp://media:secret@nas.local', p('ftp', 'nas.local', 21, 'media')],
    [':secret@nas.local', null],
    ['', null],
    ['nas local', null],
    ['@nas.local', null],
    ['nas.local:0', null],
    ['nas.local:70000', null],
    ['nas.local:', null],
    ['http://nas.local', null],
    ['nas/local', null],
    ['s3:///bucket', null],
    ['[nope]:22', null],
  ])('%s', (text, expected) => {
    expect(parseAddress(text)).toEqual(expected)
  })
})

test('describe_reads_like_the_board', () => {
  expect(describeAddress(p('ssh', 'nas.local', 22, 'admin'))).toBe(
    'SSH to nas.local as admin, port 22',
  )
  expect(describeAddress(p('ftps', 'nas.local', 21))).toBe('FTPS to nas.local, port 21')
  expect(describeAddress(p('s3', 'minio.lan', null, null, 'logs'))).toBe(
    'S3 bucket logs at minio.lan',
  )
  expect(describeAddress(p('s3', 'minio.lan', 9000))).toBe('S3 at minio.lan:9000')
})

test('address_key_matches_the_rust_rules', () => {
  expect(addressKey('NAS.local.')).toBe('nas.local')
  expect(addressKey('[::1]')).toBe('::1')
  expect(addressKey('0:0::1')).toBe('::1')
  expect(addressKey('2001:DB8::1')).toBe('2001:db8::1')
  expect(addressKey('10.0.0.1')).toBe('10.0.0.1')
})
