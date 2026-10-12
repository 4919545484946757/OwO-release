"use strict";
// Presentation only: a hidden coordinator continues observing the same team.
function createPetPresence({primary,peers}) {
  let ref=null, selected=null, desired=true, lastWindow=null;
  const alive=w=>w&&!w.isDestroyed();
  const windows=()=>[primary(),...peers().values()].filter(alive);
  function roles(){return (ref?.members||[]).filter(m=>m.runtime_binding?.kind==='agent').map(m=>m.role);}
  function primaryRole(){const all=roles();return all.find(x=>x==='lead'||x==='planner')||all[0]||null;}
  function apply(){
    const target=selected&&selected!==primaryRole()?peers().get(selected):primary();
    // Hide the old actor first; never overlap during handoff or watchdog repair.
    for(const w of windows())if(!desired||w!==target)w.hide();
    if(desired&&alive(target)){
      if(target!==lastWindow&&alive(lastWindow)){
        const [x,y]=lastWindow.getPosition();target.setPosition(x,y);
      }
      if(!target.isVisible()){
        if(target.showInactive)target.showInactive();else target.show();
      }
      lastWindow=target;
    }
  }
  return {
    bind(value){
      const changed=ref?.id!==value?.id;ref=value;
      if(changed||!roles().includes(selected))selected=primaryRole();
      apply();
    },
    update(value){
      if(!ref?.id||value?.id!==ref.id)return false;
      const active=roles().filter(role=>value.activeRoles.includes(role));
      if(['succeeded','failed','cancelled'].includes(value.status))selected=primaryRole();
      else if(active.length&&!active.includes(selected))selected=active[0];
      // Between phases, retain the departing actor until the next one starts.
      apply();return true;
    },
    setVisible(value){desired=value;apply();},apply,
    isVisible(){return windows().some(w=>w.isVisible());},
    get role(){return selected;},
  };
}
module.exports={createPetPresence};
