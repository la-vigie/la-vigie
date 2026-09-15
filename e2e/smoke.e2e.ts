describe('TASK-142 track (b) launch spike', () => {
  it('launches La Vigie under WebDriver and returns one DOM query', async () => {
    // #root is the React mount point; if it exists, the app launched under
    // WebDriver and the webview rendered — that is the entire go/no-go signal.
    const root = await $('#root')
    await expect(root).toBeExisting()
  })
})
