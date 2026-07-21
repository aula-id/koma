import assert from 'node:assert/strict'
import { baseName, dirName, parentDirPath, stripWinPrefix } from './path'

// Unix
assert.equal(baseName('/home/u/proj/src/main.rs'), 'main.rs')
assert.equal(baseName('src/main.rs'), 'main.rs')
assert.equal(baseName('main.rs'), 'main.rs')
assert.equal(baseName('/'), '/')

// Windows separators
assert.equal(baseName('C:\\Users\\62895\\Documents\\main.rs'), 'main.rs')
assert.equal(baseName('C:/Users/62895/Documents/main.rs'), 'main.rs')
assert.equal(baseName('src\\main.rs'), 'main.rs')

// Extended-length prefix (the Explore panel bug from issue #81)
assert.equal(baseName('\\\\?\\C:\\Users\\62895\\Documents\\Repository\\main.rs'), 'main.rs')
assert.equal(baseName('//?/C:/Users/62895/Documents/main.rs'), 'main.rs')
assert.equal(stripWinPrefix('\\\\?\\C:\\Users\\x\\a.rs'), 'C:\\Users\\x\\a.rs')

// dirName
assert.equal(dirName('/home/u/proj/src/main.rs'), 'home/u/proj/src')
assert.equal(dirName('C:\\Users\\62895\\Documents\\main.rs'), 'C:/Users/62895/Documents')
assert.equal(dirName('\\\\?\\C:\\Users\\62895\\Documents\\main.rs'), 'C:/Users/62895/Documents')
assert.equal(dirName('main.rs'), '')

// coding-panel parentDirPath (relative, mixed seps)
assert.equal(parentDirPath('src/nested/a.ts'), 'src/nested')
assert.equal(parentDirPath('src\\nested\\a.ts'), 'src/nested')
assert.equal(parentDirPath('a.ts'), '')
assert.equal(parentDirPath(''), '')

console.log('path.test.ts: ok')
