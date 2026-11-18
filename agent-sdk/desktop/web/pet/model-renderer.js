/* OwO 桌宠 GLB renderer.
 * Supports embedded base-color textures, UVs, vertex colors and lightweight PBR-like lighting.
 * The canvas stays transparent so the model can live in the borderless pet window.
 */
(function (global) {
  "use strict";
  const COMPONENTS={5120:1,5121:1,5122:2,5123:2,5125:4,5126:4};
  const TYPES={SCALAR:1,VEC2:2,VEC3:3,VEC4:4,MAT2:4,MAT3:9,MAT4:16};
  const IDENTITY=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1];
  function mul(a,b){const o=new Array(16).fill(0);for(let c=0;c<4;c++)for(let r=0;r<4;r++)o[c*4+r]=a[r]*b[c*4]+a[4+r]*b[c*4+1]+a[8+r]*b[c*4+2]+a[12+r]*b[c*4+3];return o;}
  function trs(n){if(n.matrix)return n.matrix.slice();const t=n.translation||[0,0,0],s=n.scale||[1,1,1],q=n.rotation||[0,0,0,1],[x,y,z,w]=q;return[(1-2*(y*y+z*z))*s[0],(2*(x*y+z*w))*s[0],(2*(x*z-y*w))*s[0],0,(2*(x*y-z*w))*s[1],(1-2*(x*x+z*z))*s[1],(2*(y*z+x*w))*s[1],0,(2*(x*z-y*w))*s[2],(2*(y*z+x*w))*s[2],(1-2*(x*x+y*y))*s[2],0,t[0],t[1],t[2],1];}
  function transform(m,p){const w=m[3]*p[0]+m[7]*p[1]+m[11]*p[2]+m[15]||1;return[(m[0]*p[0]+m[4]*p[1]+m[8]*p[2]+m[12])/w,(m[1]*p[0]+m[5]*p[1]+m[9]*p[2]+m[13])/w,(m[2]*p[0]+m[6]*p[1]+m[10]*p[2]+m[14])/w];}
  function accessor(g,b,i){const a=g.accessors[i],v=g.bufferViews[a.bufferView],n=a.count,w=TYPES[a.type],size=COMPONENTS[a.componentType],stride=v.byteStride||w*size,base=(v.byteOffset||0)+(a.byteOffset||0),dv=new DataView(b,base,Math.max(0,(n-1)*stride+w*size));const read=o=>a.componentType===5126?dv.getFloat32(o,true):a.componentType===5125?dv.getUint32(o,true):a.componentType===5123?dv.getUint16(o,true):a.componentType===5121?dv.getUint8(o):a.componentType===5122?dv.getInt16(o,true):dv.getInt8(o);const normalize=(value)=>{if(!a.normalized)return value;if(a.componentType===5121)return value/255;if(a.componentType===5123)return value/65535;if(a.componentType===5120)return Math.max(value/127,-1);if(a.componentType===5122)return Math.max(value/32767,-1);if(a.componentType===5125)return value/4294967295;return value;};const out=[];for(let x=0;x<n;x++){const row=[];for(let y=0;y<w;y++)row.push(normalize(read(x*stride+y*size)));out.push(w===1?row[0]:row);}return out;}
  async function decodeImage(blob){return new Promise((resolve,reject)=>{const image=new Image();const url=URL.createObjectURL(blob);image.onload=()=>{URL.revokeObjectURL(url);resolve(image);};image.onerror=()=>{URL.revokeObjectURL(url);reject(Error("image decode failed"));};image.src=url;});}
  async function loadGltf(url){
        const client=global.OwoApi;if(!client||typeof client.resource!=="function")throw Error("static asset client unavailable");const r=await client.resource(url,{cache:"no-store"});if(!r.ok)throw Error(`GLB HTTP ${r.status}`);const bytes=await r.arrayBuffer(),v=new DataView(bytes);if(v.getUint32(0,true)!==0x46546c67)throw Error("not a GLB");let off=12,g=null,b=new ArrayBuffer(0);while(off+8<=bytes.byteLength){const len=v.getUint32(off,true),type=v.getUint32(off+4,true);off+=8;const chunk=bytes.slice(off,off+len);off+=len;if(type===0x4e4f534a)g=JSON.parse(new TextDecoder().decode(chunk));if(type===0x004e4942)b=chunk;}if(!g)throw Error("GLB JSON missing");
        const images=await Promise.all((g.images||[]).map(async image=>{if(image.bufferView===undefined)return null;const view=g.bufferViews[image.bufferView],start=view.byteOffset||0,blob=new Blob([b.slice(start,start+view.byteLength)],{type:image.mimeType||"image/png"});return decodeImage(blob);}));
        const mats=(g.materials||[]).map(m=>{const p=m.pbrMetallicRoughness||{},tex=p.baseColorTexture&&g.textures&&g.textures[p.baseColorTexture.index],source=tex&&images[tex.source];return{color:p.baseColorFactor||[1,1,1,1],texture:source||null,roughness:Number.isFinite(p.roughnessFactor)?p.roughnessFactor:0.75,metallic:Number.isFinite(p.metallicFactor)?p.metallicFactor:0};});
    const nodes=g.nodes||[],child=new Set();nodes.forEach(n=>(n.children||[]).forEach(i=>child.add(i)));const roots=nodes.map((_,i)=>i).filter(i=>!child.has(i)),items=[];let min=[Infinity,Infinity,Infinity],max=[-Infinity,-Infinity,-Infinity];
    const walk=(ni,parent)=>{const n=nodes[ni]||{},world=mul(parent,trs(n));if(n.mesh!==undefined)for(const p of (g.meshes[n.mesh].primitives||[])){if(p.attributes?.POSITION===undefined)continue;const pos=accessor(g,b,p.attributes.POSITION),norm=p.attributes.NORMAL===undefined?pos.map(()=>[0,1,0]):accessor(g,b,p.attributes.NORMAL),uv=p.attributes.TEXCOORD_0===undefined?pos.map(()=>[0,0]):accessor(g,b,p.attributes.TEXCOORD_0),rawColor=p.attributes.COLOR_0===undefined?null:accessor(g,b,p.attributes.COLOR_0),colors=rawColor?rawColor.map(c=>c.length===3?[c[0],c[1],c[2],1]:c):pos.map(()=>[1,1,1,1]),idx=p.indices===undefined?pos.map((_,i)=>i):accessor(g,b,p.indices),tp=pos.map(q=>{const z=transform(world,q);for(let k=0;k<3;k++){min[k]=Math.min(min[k],z[k]);max[k]=Math.max(max[k],z[k]);}return z;});items.push({positions:tp,normals:norm,uvs:uv,colors,indices:idx,material:mats[p.material||0]||{color:[1,1,1,1],texture:null,roughness:0.75,metallic:0},mode:p.mode||4});}(n.children||[]).forEach(i=>walk(i,world));};roots.forEach(i=>walk(i,IDENTITY));if(!items.length)throw Error("GLB has no mesh");return{items,center:[(min[0]+max[0])/2,(min[1]+max[1])/2,(min[2]+max[2])/2],scale:2/(Math.max(max[0]-min[0],max[1]-min[1],max[2]-min[2])||1)};
  }
  function shader(gl,t,s){const x=gl.createShader(t);gl.shaderSource(x,s);gl.compileShader(x);if(!gl.getShaderParameter(x,gl.COMPILE_STATUS))throw Error(gl.getShaderInfoLog(x)||"shader compile failed");return x;}
  const VERTEX_SHADER = `
    attribute vec3 aPosition;
    attribute vec3 aNormal;
    attribute vec2 aUv;
    attribute vec4 aColor;
    uniform mat4 uMvp;
    uniform mat4 uModel;
    uniform bool uFigurine;
    uniform vec3 uHead;
    uniform vec3 uRightArm;
    uniform vec3 uLeftArm;
    uniform vec2 uTorso;
    uniform vec4 uGait;
    uniform vec3 uRightFoot;
    uniform vec3 uLeftFoot;
    uniform vec3 uRightKnee;
    uniform vec3 uLeftKnee;
    uniform vec4 uFootAngles;
    varying vec3 vNormal;
    varying vec2 vUv;
    varying vec4 vColor;

    mat3 rotateX(float angle) {
      float c = cos(angle), s = sin(angle);
      return mat3(1.0, 0.0, 0.0, 0.0, c, s, 0.0, -s, c);
    }
    mat3 rotateY(float angle) {
      float c = cos(angle), s = sin(angle);
      return mat3(c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c);
    }
    mat3 rotateZ(float angle) {
      float c = cos(angle), s = sin(angle);
      return mat3(c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0);
    }
    void bend(inout vec3 p, inout vec3 n, vec3 joint, mat3 rotation, float weight) {
      p = mix(p, joint + rotation * (p - joint), weight);
      n = mix(n, rotation * n, weight);
    }
    void stepLeg(inout vec3 p, inout vec3 n, vec3 rest, vec3 kneeShift, vec3 footShift,
                 vec2 angles, vec3 ankle, float side) {
      float leg = smoothstep(-0.012, 0.016, rest.x * side);
      float shin = 1.0 - smoothstep(-0.422, -0.29, rest.y);
      float thigh = 1.0 - smoothstep(-0.29, -0.125, rest.y);
      float boot = 1.0 - smoothstep(-0.422, -0.365, rest.y);
      vec3 hip = vec3(uGait.x, uGait.y, 0.0);
      vec3 upperShift = mix(hip, kneeShift, thigh);
      p += mix(upperShift, footShift, shin) * leg * (1.0 - smoothstep(-0.135, -0.09, rest.y));
      mat3 shoe = rotateY(angles.x) * rotateX(angles.y);
      p += (shoe * (rest - ankle) - (rest - ankle)) * boot * leg;
      n = mix(n, shoe * n, boot * leg);
    }
    void moveArm(inout vec3 p, inout vec3 n, vec3 rest, vec3 angles, float side) {
      float sleeve = smoothstep(0.105, 0.187, rest.x * side)
        * smoothstep(-0.09, -0.026, rest.y) * (1.0 - smoothstep(0.15, 0.245, rest.y))
        * smoothstep(-0.073, 0.005, rest.z);
      float elbow = sleeve * (1.0 - smoothstep(0.051, 0.139, rest.y));
      bend(p, n, vec3(0.161 * side, 0.069, 0.045), rotateX(angles.x), elbow);
      bend(p, n, vec3(0.088 * side, 0.179, 0.025),
        rotateZ(angles.y * side) * rotateX(angles.z), sleeve);
    }
    void main() {
      vec3 p = aPosition;
      vec3 n = aNormal;
      if (uFigurine) {
        // Soft joint weights in this model's bind pose. Leave the boots and lower legs planted.
        float headWeight = smoothstep(0.204, 0.304, p.y);
        float torsoWeight = smoothstep(-0.154, 0.18, p.y);
        stepLeg(p, n, aPosition, uRightKnee, uRightFoot, uFootAngles.xy, vec3(0.051, -0.422, 0.065), 1.0);
        stepLeg(p, n, aPosition, uLeftKnee, uLeftFoot, uFootAngles.zw, vec3(-0.033, -0.422, 0.086), -1.0);

        bend(p, n, vec3(0.0, 0.235, 0.052),
          rotateZ(uHead.z) * rotateY(uHead.y) * rotateX(uHead.x), headWeight);
        moveArm(p, n, aPosition, uRightArm, 1.0);
        moveArm(p, n, aPosition, uLeftArm, -1.0);
        bend(p, n, vec3(0.0, -0.101, 0.025),
          rotateZ(uGait.w) * rotateY(uGait.z) * rotateX(uTorso.x), torsoWeight);
        p += vec3(uGait.x, uGait.y, 0.0) * smoothstep(-0.125, -0.09, aPosition.y);
        p.y += torsoWeight * uTorso.y;
      }
      vNormal = normalize(mat3(uModel) * n);
      vUv = aUv;
      vColor = aColor;
      gl_Position = uMvp * vec4(p, 1.0);
    }`;

  const FRAGMENT_SHADER = `
    precision mediump float;
    uniform vec4 uColor;
    uniform vec3 uLight;
    uniform sampler2D uTexture;
    uniform bool uUseTexture;
    uniform float uRoughness;
    uniform bool uStudio;
    varying vec3 vNormal;
    varying vec2 vUv;
    varying vec4 vColor;

    void main() {
      vec4 texel = uUseTexture ? texture2D(uTexture, vUv) : vec4(1.0);
      vec4 base = uColor * vColor * texel;
      vec3 normal = normalize(vNormal);
      vec3 light = normalize(uLight);
      float key = max(dot(normal, light), 0.0);
      if (uStudio) {
        vec3 albedo = pow(base.rgb, vec3(2.2));
        float fill = max(dot(normal, normalize(vec3(0.70, 0.12, 0.65))), 0.0);
        float rim = pow(max(dot(normal, normalize(vec3(0.35, 0.65, -0.68))), 0.0), 3.0);
        vec3 halfVector = normalize(light + vec3(0.0, 0.0, 1.0));
        float highlight = pow(max(dot(normal, halfVector), 0.0), mix(44.0, 12.0, uRoughness));
        vec3 lit = albedo * (vec3(0.58, 0.60, 0.68)
          + key * vec3(0.36, 0.32, 0.27) + fill * vec3(0.12, 0.14, 0.17));
        lit += vec3(0.018, 0.024, 0.040) * rim
          + vec3(0.033, 0.030, 0.029) * highlight;
        gl_FragColor = vec4(pow(lit, vec3(1.0 / 2.2)), base.a);
      } else {
        float diffuse = 0.32 + 0.68 * key;
        float sheen = (1.0 - uRoughness) * 0.12 * pow(key, 5.0);
        gl_FragColor = vec4(base.rgb * (diffuse + sheen), base.a);
      }
    }`;

  function program(gl) {
    const p = gl.createProgram();
    gl.attachShader(p, shader(gl, gl.VERTEX_SHADER, VERTEX_SHADER));
    gl.attachShader(p, shader(gl, gl.FRAGMENT_SHADER, FRAGMENT_SHADER));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw Error(gl.getProgramInfoLog(p) || "shader link failed");
    return p;
  }

  // Joint angles in radians, ordered by neck, torso, then the two sleeves.
  const STILL = [0, 0, 0];
  function pose(head = STILL, torso = 0, right = STILL, left = STILL) {
    return [...head, torso, ...right, ...left];
  }
  const REST = pose();
  const POSES = {
    thinking: [
      [0, REST],
      [0.64, pose([0.15, -0.20, -0.15], 0.04, [-0.11, 0.04, -0.04])],
    ],
    speaking: [
      [0, REST],
      [0.28, pose([-0.04, 0.10, 0.03], -0.025, [-0.30, 0.18, -0.10])],
      [0.61, pose([0.085, 0.06, 0.02], 0.018, [-0.57, 0.44, -0.16], [-0.12, 0.08, 0])],
      [1.04, pose([-0.035, 0, 0.01], 0, [-0.19, 0.15, -0.07])],
      [1.52, REST],
    ],
    alert: [
      [0, REST],
      [0.24, pose([-0.13, 0, 0.04], -0.07, [-0.14, 0.08, -0.09], [-0.10, 0.05, -0.06])],
      [0.85, pose([-0.05, 0, 0], -0.024, [-0.045, 0.03, -0.04])],
    ],
    wave: [
      [0, REST],
      [0.27, pose([0.04, 0.09, -0.04], 0.02, [-0.24, 0.18, -0.07])],
      [0.58, pose([-0.045, 0.08, 0.07], -0.02, [-0.76, 0.55, -0.23])],
      [0.79, pose([-0.04, 0.04, 0.05], -0.016, [-0.44, 0.59, -0.23])],
      [1.02, pose([0, 0.03, 0.03], 0, [-0.70, 0.49, -0.19])],
      [1.55, REST],
    ],
    bow: [
      [0, REST],
      [0.28, pose([0.11, 0, -0.035], 0.035)],
      [0.59, pose([0.16, 0.035, -0.04], 0.16, [-0.12, 0.06, 0.03], [-0.14, 0.08, 0.02])],
      [0.76, pose([0.14, 0.04, -0.035], 0.14, [-0.09, 0.05, 0.02], [-0.11, 0.07, 0.02])],
      [1.32, REST],
    ],
    glance: [
      [0, REST],
      [0.62, pose([-0.022, 0.27, 0.075], -0.015)],
      [1.42, pose([0.018, 0.24, 0.065], -0.01)],
      [2.00, pose([0.028, -0.16, -0.035], 0.007)],
      [2.55, pose([0.035, -0.12, -0.04], 0.006)],
      [3.15, REST],
    ],
    stretch: [
      [0, REST],
      [0.55, pose([0.08, -0.03, -0.05], 0.045, [-0.12, 0.14, -0.06], [-0.15, 0.19, -0.06])],
      [1.43, pose([-0.12, 0.04, 0.055], -0.11, [-0.43, 0.54, -0.22], [-0.35, 0.49, -0.16])],
      [2.06, pose([-0.08, 0.035, 0.04], -0.075, [-0.35, 0.48, -0.16], [-0.29, 0.43, -0.12])],
      [3.02, REST],
    ],
    sleeve: [
      [0, REST],
      [0.49, pose([0.17, -0.17, -0.055], 0.045, STILL, [-0.19, 0.11, -0.05])],
      [0.98, pose([0.20, -0.22, -0.095], 0.065, [-0.07, 0.05, 0], [-0.54, 0.30, -0.19])],
      [1.47, pose([0.16, -0.20, -0.06], 0.048, STILL, [-0.41, 0.25, -0.17])],
      [2.30, REST],
    ],
  };
  const IDLE_ACTIVITIES = [
    { start: 3, track: POSES.glance },
    { start: 17, track: POSES.stretch },
    { start: 28, track: POSES.sleeve },
  ];
  const IDLE_CYCLE_SECONDS = 38;

  function samplePose(track, seconds) {
    if (!track) return REST;
    for (let i = 1; i < track.length; i++) {
      const [end, target] = track[i];
      if (seconds > end) continue;
      const [start, source] = track[i - 1];
      const t = Math.max(0, (seconds - start) / (end - start));
      const eased = t * t * (3 - 2 * t);
      return source.map((value, j) => value + (target[j] - value) * eased);
    }
    return track.at(-1)[1];
  }

  // The route is authored as footsteps. Each footprint stays in stage coordinates
  // until that shoe lifts, including through the turn at the end of the path.
  const WALK_START = 7.5;
  const HIP = [[0.039, -0.125, 0.048], [-0.029, -0.125, 0.072]];
  const ANKLE = [[0.051, -0.422, 0.065], [-0.033, -0.422, 0.086]];
  const STEP_LENGTHS = [0.169, 0.134];
  const WALK_STEPS = [
    // End time, root X, root yaw, moving shoe, footprint X and yaw, lift height.
    [0.25, 0.008, 0.12, null],
    [0.69, 0.067, 0.79, 0, 0.122, 0.99, 0.071],
    [1.11, 0.195, 1.13, 1, 0.258, 1.16, 0.092],
    [1.53, 0.317, 1.12, 0, 0.362, 1.05, 0.095],
    [1.96, 0.344, 0.60, 1, 0.314, -0.19, 0.077],
    [2.42, 0.335, -0.42, 0, 0.326, -0.57, 0.068],
    [2.86, 0.262, -1.00, 1, 0.203, -1.10, 0.086],
    [3.30, 0.133, -1.15, 0, 0.079, -1.14, 0.089],
    [3.74, 0.026, -0.75, 1, 0.006, -0.52, 0.080],
    [4.14, 0, -0.20, 0, 0, 0, 0.066],
    [4.50, 0, 0, 1, 0, 0, 0.055],
    [4.72, 0, 0, null],
  ];
  const WALK_END = WALK_STEPS.at(-1)[0];
  // Root X, yaw, pelvis X/Y, upper-body yaw/roll, head lead, arm swing,
  // then two footprint offsets (X/Y/Z) and local yaw/pitch for each shoe.
  const WALK_REST = new Array(18).fill(0);
  const lerp = (a, b, t) => a + (b - a) * t;
  function ease(t) {
    return t * t * (3 - 2 * t);
  }

  function footprint(ankle, root, yaw, scale, center) {
    const x = ankle[0] - center[0], z = ankle[2] - center[2];
    return [root + scale * (x * Math.cos(yaw) + z * Math.sin(yaw)),
      scale * (z * Math.cos(yaw) - x * Math.sin(yaw)), yaw];
  }

  function sampleWalk(seconds, side, scale, center) {
    const feet = ANKLE.map(ankle => footprint(ankle, 0, 0, scale, center));
    let start = 0, root = 0, yaw = 0;
    for (const [end, targetRoot, targetYaw, shoe, plantRoot, plantYaw, height] of WALK_STEPS) {
      const moving = side > 0 || shoe === null ? shoe : 1 - shoe;
      const plant = moving === null ? null : footprint(ANKLE[moving], plantRoot * side, plantYaw * side, scale, center);
      if (seconds > end) {
        if (plant) feet[moving] = plant;
        start = end;
        root = targetRoot * side;
        yaw = targetYaw * side;
        continue;
      }
      const t = (seconds - start) / (end - start);
      const eased = ease(t);
      const x = lerp(root, targetRoot * side, lerp(t, eased, 0.55));
      const heading = lerp(yaw, targetYaw * side, eased);
      const step = Math.sin(Math.PI * t);
      const sway = moving === 0 ? -step : step;
      const weight = moving === null ? 0 : ease(Math.min(1, (seconds - 0.25) / 0.45, (4.50 - seconds) / 0.42));
      const hipX = 0.020 * sway * weight;
      const hipY = weight * (-0.018 + step * 0.012);
      const arm = Math.sin(Math.PI * (t - 0.5)) * (moving === 0 ? 1 : -1) * weight;
      const local = feet.flatMap(([fromX, fromZ, fromYaw], i) => {
        const swing = i === moving;
        const px = swing ? lerp(fromX, plant[0], eased) : fromX;
        const pz = swing ? lerp(fromZ, plant[1], eased) : fromZ;
        const footYaw = swing ? lerp(fromYaw, plant[2], eased) : fromYaw;
        const dx = (px - x) / scale;
        const dz = pz / scale;
        return [dx * Math.cos(heading) - dz * Math.sin(heading) + center[0] - ANKLE[i][0],
          swing ? height * step ** 1.6 / scale : 0,
          dx * Math.sin(heading) + dz * Math.cos(heading) + center[2] - ANKLE[i][2],
          footYaw - heading,
          swing ? -0.21 * Math.sin(2 * Math.PI * t) * step : 0];
      });
      return [x, heading, hipX, hipY, arm * 0.10, hipX * -3.5,
        (targetYaw * side - heading) * 0.23 * step, arm * 0.19, ...local];
    }
    return WALK_REST;
  }

  function solveKnee(hip, ankle) {
    const delta = ankle.map((value, i) => value - hip[i]);
    const length = Math.hypot(...delta);
    const axis = delta.map(value => value / length);
    const [thigh, shin] = STEP_LENGTHS;
    const reach = Math.min(thigh + shin, length);
    const along = (reach ** 2 + thigh ** 2 - shin ** 2) / (2 * reach);
    const bend = Math.sqrt(Math.max(0, thigh ** 2 - along ** 2));
    // The knees bend toward the character's forward axis.
    const forward = [-axis[0] * axis[2], -axis[1] * axis[2], 1 - axis[2] ** 2];
    const width = Math.hypot(...forward);
    return hip.map((value, i) => value + axis[i] * along + forward[i] * bend / width);
  }
  const REST_KNEES = HIP.map((hip, i) => solveKnee(hip, ANKLE[i]));

  function applyGait(gl, loc, walk) {
    const hipX = walk[2], hipY = walk[3];
    gl.uniform4f(loc.gait, hipX, hipY, walk[4], walk[5]);
    const shifts = [walk.slice(8, 13), walk.slice(13, 18)];
    for (const [i, footLoc, kneeLoc] of [[0, loc.rightFoot, loc.rightKnee], [1, loc.leftFoot, loc.leftKnee]]) {
      const shift = shifts[i];
      const hip = [HIP[i][0] + hipX, HIP[i][1] + hipY, HIP[i][2]];
      const ankle = ANKLE[i].map((value, axis) => value + shift[axis]);
      const knee = solveKnee(hip, ankle);
      gl.uniform3f(footLoc, shift[0], shift[1], shift[2]);
      gl.uniform3f(kneeLoc, ...knee.map((value, axis) => value - REST_KNEES[i][axis]));
    }
    gl.uniform4f(loc.footAngles, shifts[0][3], shifts[0][4], shifts[1][3], shifts[1][4]);
  }

  function createFigurineMotion() {
    const reducedMotion = global.matchMedia("(prefers-reduced-motion: reduce)");
    let state = "idle", entered = performance.now(), touched = -Infinity;
    let last = REST, previous = REST, changed = entered, touchTrack = POSES.bow;
    let walk = WALK_REST, previousWalk = WALK_REST;
    function transition(now) {
      previous = last;
      previousWalk = walk;
      changed = now;
    }
    return {
      reset(next) {
        state = next;
        entered = changed = performance.now();
        touched = -Infinity;
        last = previous = REST;
        walk = previousWalk = WALK_REST;
        touchTrack = POSES.bow;
      },
      setState(next) {
        if (state === next) return;
        state = next;
        entered = performance.now();
        if (next === "alert") touched = -Infinity;
        transition(entered);
      },
      pet() {
        touched = performance.now();
        touchTrack = touchTrack === POSES.wave ? POSES.bow : POSES.wave;
        transition(touched);
      },
      apply(gl, loc, now, allowWalk, model) {
        let target = REST, nextWalk = WALK_REST, breath = 0, side = 1;
        if (!reducedMotion.matches) {
          const elapsed = Math.max(0, (now - entered) / 1000);
          const touchTime = (now - touched) / 1000;
          const touchEnd = touchTrack.at(-1)[0];
          if (touchTime <= touchEnd) {
            target = samplePose(touchTrack, touchTime);
          } else if (state === "idle") {
            const phase = elapsed % IDLE_CYCLE_SECONDS;
            const patrol = phase - WALK_START;
            if (allowWalk && patrol >= 0 && patrol <= WALK_END) {
              const direction = Math.floor(elapsed / IDLE_CYCLE_SECONDS) % 2 ? -1 : 1;
              nextWalk = sampleWalk(patrol, direction, model.scale, model.center);
              const swing = nextWalk[7];
              target = pose([-0.025 * Math.abs(swing), nextWalk[6], -nextWalk[5] * 0.38], -0.13 * Math.abs(swing),
                [Math.min(0, swing * 0.68), 0.025, swing], [Math.min(0, -swing * 0.68), 0.025, -swing]);
            }
            const activity = IDLE_ACTIVITIES.find(({ start, track }) =>
              phase >= start && phase <= start + track.at(-1)[0]);
            if (activity) {
              target = samplePose(activity.track, phase - activity.start);
              if (activity.track === POSES.glance && Math.floor(elapsed / IDLE_CYCLE_SECONDS) % 2) side = -1;
            }
          } else {
            target = samplePose(POSES[state], elapsed);
          }
          const blend = Math.min(1, Math.max(0, (now - changed) / 190));
          const resume = touchTime > touchEnd ? Math.min(1, (touchTime - touchEnd) / 0.24) : 1;
          target = target.map((value, i) => {
            const direction = i === 1 || i === 2 ? side : 1;
            return previous[i] + (value * direction * resume - previous[i]) * blend;
          });
          nextWalk = nextWalk.map((value, i) => previousWalk[i] + (value * resume - previousWalk[i]) * blend);
          breath = (1 - Math.cos(now / 760)) / 2;
        }
        last = target;
        walk = nextWalk;
        const [pitch, turn, tilt, lean, rightElbow, rightLift, rightShoulder, leftElbow, leftLift, leftShoulder] = last;
        gl.uniform3f(loc.head, pitch + breath * 0.007, turn, tilt);
        gl.uniform3f(loc.rightArm, rightElbow, rightLift, rightShoulder);
        gl.uniform3f(loc.leftArm, leftElbow, leftLift, leftShoulder);
        gl.uniform2f(loc.torso, lean - breath * 0.011, breath * 0.008);
        applyGait(gl, loc, walk);
        return walk;
      },
    };
  }
  function perspective(f,a,n,far){const q=1/Math.tan(f/2),nf=1/(n-far);return[q/a,0,0,0,0,q,0,0,0,0,(far+n)*nf,-1,0,0,2*far*n*nf,0];}
  function translate(x,y,z){return[1,0,0,0,0,1,0,0,0,0,1,0,x,y,z,1];}
  function scale3(x){return[x,0,0,0,0,x,0,0,0,0,x,0,0,0,0,1];}
  function ry(a){const c=Math.cos(a),s=Math.sin(a);return[c,0,-s,0,0,1,0,0,s,0,c,0,0,0,0,1];}
  function rz(a){const c=Math.cos(a),s=Math.sin(a);return[c,s,0,0,-s,c,0,0,0,0,1,0,0,0,0,1];}
  function rx(a){const c=Math.cos(a),s=Math.sin(a);return[1,0,0,0,0,c,s,0,0,-s,c,0,0,0,0,1];}
  function create(canvas,{onOffset}={}){if(!canvas||typeof canvas.getContext!=="function")return null;let gl;try{gl=canvas.getContext("webgl2",{alpha:true,antialias:true,premultipliedAlpha:true})||canvas.getContext("webgl",{alpha:true,antialias:true,premultipliedAlpha:true});}catch{return null;}if(!gl)return null;const p=program(gl),loc={pos:gl.getAttribLocation(p,"aPosition"),normal:gl.getAttribLocation(p,"aNormal"),uv:gl.getAttribLocation(p,"aUv"),colorAttr:gl.getAttribLocation(p,"aColor"),mvp:gl.getUniformLocation(p,"uMvp"),model:gl.getUniformLocation(p,"uModel"),color:gl.getUniformLocation(p,"uColor"),light:gl.getUniformLocation(p,"uLight"),texture:gl.getUniformLocation(p,"uTexture"),useTexture:gl.getUniformLocation(p,"uUseTexture"),roughness:gl.getUniformLocation(p,"uRoughness"),studio:gl.getUniformLocation(p,"uStudio"),figurine:gl.getUniformLocation(p,"uFigurine"),head:gl.getUniformLocation(p,"uHead"),rightArm:gl.getUniformLocation(p,"uRightArm"),leftArm:gl.getUniformLocation(p,"uLeftArm"),torso:gl.getUniformLocation(p,"uTorso"),gait:gl.getUniformLocation(p,"uGait"),rightFoot:gl.getUniformLocation(p,"uRightFoot"),leftFoot:gl.getUniformLocation(p,"uLeftFoot"),rightKnee:gl.getUniformLocation(p,"uRightKnee"),leftKnee:gl.getUniformLocation(p,"uLeftKnee"),footAngles:gl.getUniformLocation(p,"uFootAngles")};let model=null,url="",state="idle",raf=0,started=performance.now(),dead=false,lastDraw=0,rotX=0,rotY=0,studio=false,figurine=false,modelRequest=0,offset=0,walking=false;const motion=createFigurineMotion(),buffers=new WeakMap(),textureCache=new WeakMap();canvas.addEventListener("webglcontextlost",event=>{event.preventDefault();setTimeout(()=>global.location.reload(),250);},{once:true});function resize(){const r=canvas.getBoundingClientRect(),d=Math.min(2,global.devicePixelRatio||1),w=Math.max(1,Math.round(r.width*d)),h=Math.max(1,Math.round(r.height*d));if(canvas.width!==w||canvas.height!==h){canvas.width=w;canvas.height=h;}gl.viewport(0,0,canvas.width,canvas.height);}function prepTexture(image){if(!image)return null;if(textureCache.has(image))return textureCache.get(image);const tex=gl.createTexture();gl.bindTexture(gl.TEXTURE_2D,tex);gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL,false);gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA,gl.RGBA,gl.UNSIGNED_BYTE,image);const mipmap=studio&&[image.width,image.height].every(size=>size>0&&(size&(size-1))===0);if(mipmap)gl.generateMipmap(gl.TEXTURE_2D);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,mipmap?gl.LINEAR_MIPMAP_LINEAR:gl.LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.LINEAR);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);textureCache.set(image,tex);return tex;}function prep(i){if(buffers.has(i))return buffers.get(i);const pos=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,pos);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(i.positions.flat()),gl.STATIC_DRAW);const nor=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,nor);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(i.normals.flat()),gl.STATIC_DRAW);const uv=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,uv);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(i.uvs.flat()),gl.STATIC_DRAW);const col=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,col);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(i.colors.flat()),gl.STATIC_DRAW);let maxIndex=0;for(const value of i.indices)maxIndex=Math.max(maxIndex,value);const indexType=maxIndex>65535&&gl.UNSIGNED_INT?gl.UNSIGNED_INT:gl.UNSIGNED_SHORT;const indexArray=indexType===gl.UNSIGNED_INT?new Uint32Array(i.indices):new Uint16Array(i.indices);const idx=gl.createBuffer();gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,idx);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,indexArray,gl.STATIC_DRAW);const x={pos,nor,uv,col,idx,indexType,count:i.indices.length,texture:prepTexture(i.material.texture)};buffers.set(i,x);return x;}function updateOffset(next) {
    if (offset === next) return;
    offset = next;
    onOffset?.(next);
  }
  function draw(now){if(dead)return;if(now-lastDraw<(walking?15:figurine?30:50)){raf=global.requestAnimationFrame(draw);return;}lastDraw=now;try{resize();gl.clearColor(0,0,0,0);gl.clear(gl.COLOR_BUFFER_BIT|gl.DEPTH_BUFFER_BIT);gl.enable(gl.DEPTH_TEST);gl.enable(gl.BLEND);gl.blendFunc(gl.SRC_ALPHA,gl.ONE_MINUS_SRC_ALPHA);if(model){
    gl.useProgram(p);
    const defaultView = Math.abs(rotX) < 0.12 && Math.abs(Math.sin(rotY / 2)) < 0.08;
    const [travel, facing] = figurine ? motion.apply(gl, loc, now, defaultView, model) : WALK_REST;
    walking = Boolean(travel || facing);
    const pixelsPerUnit = canvas.clientHeight / (2 * Math.tan(Math.PI / 8.4) * 3.15);
    updateOffset(Math.round(travel * pixelsPerUnit));
    const t=(now-started)/1000,bob=Math.sin(t*(state==="thinking"?3.4:1.8))*0.05,yaw=Math.sin(t*(state==="idle"?0.65:1.3))*0.32+rotY,roll=state==="alert"?Math.sin(t*8)*0.08:state==="speaking"?Math.sin(t*7)*0.06:0,m=mul(translate(travel,figurine?0:bob+0.12,0),mul(rx(rotX),mul(rz(figurine?0:roll),mul(ry(figurine?rotY+facing:yaw),mul(scale3(model.scale),translate(-model.center[0],-model.center[1],-model.center[2])))))),v=translate(0,0,-3.15),pr=perspective(Math.PI/4.2,canvas.width/Math.max(1,canvas.height),0.1,20),mvp=mul(pr,mul(v,m));gl.uniformMatrix4fv(loc.mvp,false,new Float32Array(mvp));gl.uniformMatrix4fv(loc.model,false,new Float32Array(m));gl.uniform3f(loc.light,-0.45,0.8,0.7);gl.uniform1i(loc.studio,studio?1:0);gl.uniform1i(loc.figurine,figurine?1:0);for(const i of model.items){const q=prep(i);gl.bindBuffer(gl.ARRAY_BUFFER,q.pos);gl.enableVertexAttribArray(loc.pos);gl.vertexAttribPointer(loc.pos,3,gl.FLOAT,false,0,0);gl.bindBuffer(gl.ARRAY_BUFFER,q.nor);gl.enableVertexAttribArray(loc.normal);gl.vertexAttribPointer(loc.normal,3,gl.FLOAT,false,0,0);gl.bindBuffer(gl.ARRAY_BUFFER,q.uv);gl.enableVertexAttribArray(loc.uv);gl.vertexAttribPointer(loc.uv,2,gl.FLOAT,false,0,0);gl.bindBuffer(gl.ARRAY_BUFFER,q.col);gl.enableVertexAttribArray(loc.colorAttr);gl.vertexAttribPointer(loc.colorAttr,4,gl.FLOAT,false,0,0);gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,q.idx);const c=i.material.color||[1,1,1,1];gl.uniform4f(loc.color,c[0],c[1],c[2],c[3]===undefined?1:c[3]);gl.uniform1f(loc.roughness,i.material.roughness||0.75);gl.uniform1i(loc.useTexture,q.texture?1:0);if(q.texture){gl.activeTexture(gl.TEXTURE0);gl.bindTexture(gl.TEXTURE_2D,q.texture);gl.uniform1i(loc.texture,0);}gl.drawElements(i.mode===4?gl.TRIANGLES:gl.TRIANGLE_STRIP,q.count,q.indexType,0);}}}catch(error){console.error("OwO 3D draw failed",error);}raf=global.requestAnimationFrame(draw);}function start(){if(!raf){started=performance.now();raf=global.requestAnimationFrame(draw);}}return{async setModel(next,options){
    if(next===url){start();return;}
    url=next;
    const request=++modelRequest;
    let loaded;
    try{
      loaded=await loadGltf(next);
    }catch(error){
      if(request!==modelRequest||dead)return;
      url="";
      throw error;
    }
    if(request!==modelRequest||dead)return;
    loaded.scale*=Number(options?.scale)||1.0;
    model=loaded;
    rotX=0;
    rotY=0;
    studio=options?.studio===true;
    figurine=options?.motion==="figurine";
    if(figurine)motion.reset(state);
    start();
  },setState(next){state=next||"idle";if(figurine)motion.setState(state);start();},pet(){if(figurine)motion.pet();},rotate(dx,dy){rotY+=Number(dx)||0;rotX=Math.max(-1.1,Math.min(1.1,rotX+(Number(dy)||0)));start();},clear(){modelRequest++;model=null;url="";figurine=false;updateOffset(0);},destroy(){dead=true;if(raf)global.cancelAnimationFrame(raf);raf=0;gl.getExtension("WEBGL_lose_context")?.loseContext();}};}
  global.OwoModelRenderer={create};
})(globalThis);
