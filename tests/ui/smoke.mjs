// Run against the production bundle. Only this loopback test server supplies mock IPC.
import http from 'node:http'
import { spawn } from 'node:child_process'
import { readFile, mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { once } from 'node:events'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../../dist/', import.meta.url))
const html = (await readFile(path.join(root, 'index.html'), 'utf8')).replace(
  '</head>',
  '<script src="/test-fixture.js"></script></head>',
)
const fixture = await readFile(new URL('fixture.js', import.meta.url))
const mime = { '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png' }
let complete, failure
const result = new Promise((resolve, reject) => {
  complete = resolve
  failure = reject
})
const server = http.createServer(async (request, response) => {
  const url = request.url
  if (url === '/report' && request.method === 'POST') {
    let body = ''
    for await (const chunk of request) body += chunk
    response.end('ok')
    const report = JSON.parse(body)
    if (report.error) failure(new Error(report.error))
    else complete(report.tests)
  } else if (url === '/') {
    response.setHeader('Content-Type', 'text/html')
    response.end(html)
  } else if (url === '/test-fixture.js') {
    response.setHeader('Content-Type', 'text/javascript')
    response.end(fixture)
  } else if (/^\/assets\/[A-Za-z0-9._-]+$/.test(url)) {
    try {
      response.setHeader('Content-Type', mime[path.extname(url)] || 'application/octet-stream')
      response.end(await readFile(path.join(root, url)))
    } catch {
      response.writeHead(404).end()
    }
  } else response.writeHead(404).end()
})
const profile = await mkdtemp(path.join(tmpdir(), 'repo-manager-browser-'))
let browser, timer
try {
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  browser = spawn(
    'firefox',
    [
      '--headless',
      '--no-remote',
      '--new-instance',
      '--profile',
      profile,
      `http://127.0.0.1:${server.address().port}/`,
    ],
    { stdio: 'ignore' },
  )
  browser.once('error', failure)
  browser.once('exit', (code) => failure(new Error(`Test browser exited (${code}).`)))
  timer = setTimeout(() => failure(new Error('Browser interaction test timed out.')), 60000)
  const passed = await result
  for (const name of passed) console.log('PASS', name)
} finally {
  clearTimeout(timer)
  if (browser && browser.exitCode === null && browser.pid) {
    const exited = once(browser, 'exit')
    browser.kill()
    await exited
  }
  server.closeAllConnections()
  server.close()
  await rm(profile, { recursive: true, force: true })
}
