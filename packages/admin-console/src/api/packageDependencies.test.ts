import { describe, expect, it } from 'vitest'
import { dependencyList } from './packages'

describe('dependencyList', () => {
  it('passes the Array shape through', () => {
    const deps = [{ name: 'a', version: '*' }]
    expect(dependencyList(deps)).toEqual(deps)
  })

  it('reads the old Object-keyed-by-index shape in order', () => {
    expect(
      dependencyList({
        '1': { name: 'b', version: '2' },
        '0': { name: 'a', version: '1' },
        '10': { name: 'k', version: '11' },
      })
    ).toEqual([
      { name: 'a', version: '1' },
      { name: 'b', version: '2' },
      { name: 'k', version: '11' },
    ])
  })

  it('treats a missing value as no dependencies', () => {
    expect(dependencyList(undefined)).toBeUndefined()
    expect(dependencyList(null)).toBeUndefined()
  })
})
