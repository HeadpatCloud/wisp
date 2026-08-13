import { expect, test } from 'vitest'
import { encodePath, remoteUrl, urlHost } from './urls'

test('urlHost brackets bare IPv6 literals only', () => {
  expect(urlHost('2001:db8::1')).toBe('[2001:db8::1]')
  expect(urlHost('[2001:db8::1]')).toBe('[2001:db8::1]')
  expect(urlHost('example.com')).toBe('example.com')
  expect(urlHost('10.0.0.1')).toBe('10.0.0.1')
})

test('encodePath escapes segments but keeps separators', () => {
  expect(encodePath('/a b/c#d.txt')).toBe('/a%20b/c%23d.txt')
})

// Without brackets the address's own colons would be read as the port.
test('remoteUrl stays parseable for an IPv6 host', () => {
  const url = remoteUrl('sftp', { user: 'root', host: '2001:db8::1', port: 22 }, '/etc/hosts')
  expect(url).toBe('sftp://root@[2001:db8::1]:22/etc/hosts')
  expect(new URL(url).port).toBe('22')
  expect(new URL(url).hostname).toBe('[2001:db8::1]')
})

test('remoteUrl leaves hostnames and IPv4 alone', () => {
  expect(remoteUrl('sftp', { user: 'me', host: 'example.com', port: 2222 }, 'rel')).toBe(
    'sftp://me@example.com:2222/rel',
  )
})
