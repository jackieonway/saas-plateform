import assert from 'node:assert/strict'
import test from 'node:test'
import fs from 'node:fs'
import {componentFixture} from './helpers/component-fixture.mjs'

test('retired partner intake and invitation modules cannot expose a management UI',()=>{
  for(const file of ['src/pages/system/oauth/PartnerIntakePanel.tsx','src/api/identity-review.ts']) {
    assert.equal(fs.existsSync(new URL('../'+file,import.meta.url)),false)
  }
})
test('client transport has no retired consent-group or signing-key mutation methods',()=>{
  const {identityClientsApi}=componentFixture('src/api/identity-clients.ts',['identityClientsApi']).exports
  for(const method of ['consentGroups','createConsentGroup','publishKey','activateKey','invite','invitations']) assert.equal(identityClientsApi[method],undefined)
  for(const method of ['list','create','update','rotateCredential','revokeCredentials']) assert.equal(typeof identityClientsApi[method],'function')
})
