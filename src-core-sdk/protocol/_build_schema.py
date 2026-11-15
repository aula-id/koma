"""Maintainer helper: regenerate schema.json, no third-party dependencies."""
import json
from pathlib import Path

def obj(props, required=None):
    return {'type':'object','properties':props,'required':list(props) if required is None else required}
def ref(n): return {'$ref':'#/$defs/'+n}
def enum(*v): return {'enum':list(v)}
s={'type':'string'}; ident={'type':'string','minLength':1}; boolean={'type':'boolean'}
nullable={'type':['string','null']}
revision={'type':'integer','minimum':1,'maximum':9007199254740991}
offset={'type':'integer','minimum':0,'maximum':9007199254740991}
def nullref(n): return {'anyOf':[ref(n),{'type':'null'}]}
d={}
d['policy']=obj({'plan':enum('allow','forbid'),'approval':enum('default','reject')})
d['turn']=obj({'id':ident,'state':enum('running','finished','interrupted','failed'),'error':nullable})
d['message']=obj({'role':enum('system','user','assistant','tool'),'content':s})
d['pending_approval']=obj({'call_id':ident,'reason':nullable})
d['gate']=obj({'kind':enum('tool','plan','mission','unknown'),'reason':nullable,'supported':boolean})
d['state']=obj({'session_id':ident,'revision':revision,'external_approvals':boolean,'pending_approval':nullref('pending_approval'),'gate':nullref('gate'),'cwd':ident,'model_id':s,'policy':ref('policy'),'turn':nullref('turn'),'working':boolean,'awaiting_approval':boolean,'approval_reason':nullable,'pending_count':{'type':'integer','minimum':0,'maximum':5},'messages':{'type':'array','items':ref('message')},'streaming':nullable})
d['attachment']={'oneOf':[obj({'kind':{'const':'image_file'},'path':ident}),obj({'kind':{'const':'pasted_text'},'text':s,'name':nullable},['kind','text'])]}
input_props={'text':{'type':'string','minLength':1},'attachments':{'type':'array','maxItems':16,'items':ref('attachment')}}
roles=('main','awareness','safeguard','compactor','planner')
provider_in=obj({'name':ident,'endpoint':ident,'api_type':enum('openai','anthropic'),'api_key':{'type':['string','null'],'minLength':1}},['name','endpoint','api_type'])
assignment=obj({'provider':ident,'model_id':ident,'route':ident},['provider','model_id'])
params={'create':obj({'cwd':ident,'model_id':ident,'policy':ref('policy'),'external_approvals':boolean},['cwd']),'resume':obj({'session_id':ident,'policy':ref('policy'),'external_approvals':boolean},['session_id']),'state':obj({}),'submit':obj(input_props,['text']),'enqueue':obj(input_props,['text']),'interrupt':obj({'turn_id':ident}),'interrupt_and_submit':obj({'turn_id':ident,**input_props},['turn_id','text']),'approve':obj({'call_id':ident,'approve':boolean}),'configure':obj({'providers':{'type':'array','maxItems':32,'items':provider_in},'models':obj({role:assignment for role in roles},[])},[]),'detach':obj({})}
d['request']={'oneOf':[obj({'id':ident,'method':{'const':m},'params':p}) for m,p in params.items()]}
d['reason']=obj({'code':ident,'message':s})
d['turn_result']=obj({'disposition':enum('started','queued','rejected','interrupted'),'turn_id':nullable,'reason':nullref('reason')})
d['replacement_result']=obj({'disposition':enum('started','rejected'),'turn_id':nullable,'reason':nullref('reason'),'interrupted_turn_id':nullable})
d['error']=obj({'code':ident,'message':s,'delivery':enum('not_sent','unknown')})
d['configure_result']=obj({'providers':{'type':'array','items':obj({'name':ident,'uuid':ident})},'models':obj({role:obj({'uuid':ident,'model_id':ident,'provider':ident}) for role in roles},[])},['providers','models'])
d['hello']=obj({'type':{'const':'hello'},'protocol':{'const':1},'binary':obj({'path':ident,'version':ident}),'diagnostics':{'type':'array','items':s},'capabilities':{'type':'array','uniqueItems':True,'items':enum(*params)}})
d['response']={'oneOf':[obj({'type':{'const':'response'},'id':ident,'result':{'oneOf':[obj({'state':ref('state')}),ref('turn_result'),ref('replacement_result'),obj({'approved':boolean}),obj({'detached':{'const':True}}),ref('configure_result')]}}),obj({'type':{'const':'response'},'id':ident,'error':ref('error')})]}
# Forbid only mutually exclusive envelope keys; additive fields remain permitted.
for variant in d['response']['oneOf']:
    variant['not']={'required':['error' if 'result' in variant['properties'] else 'result']}
# Generic result alternatives overlap because forward-compatible properties are allowed.
d['response']['oneOf'][0]['properties']['result']['anyOf']=d['response']['oneOf'][0]['properties']['result'].pop('oneOf')
d['event']={'oneOf':[obj({'type':{'const':'event'},'event':{'const':name},'seq':revision,**props}) for name,props in {
    'state':{'state':ref('state'),'resynced':boolean},
    'output_append':{'revision':revision,'offset':offset,'text':ident},
    'turn':{'revision':revision,'turn':nullref('turn')},
}.items()]}
schema={'$schema':'https://json-schema.org/draft/2020-12/schema','$id':'https://koma.dev/sdk/protocol/v1/schema.json','title':'Koma public SDK JSONL protocol v1','oneOf':[ref(n) for n in ('request','hello','response','event')],'$defs':d}
Path(__file__).with_name('schema.json').write_text(json.dumps(schema,indent=2)+'\n')
