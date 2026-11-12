// A deck.gl layer extension that starts linking a layer's GPU program and returns at once, for the
// hidden canvas that prepares the map's programs while the snapshot loads (ProgramWarmup.tsx).
//
// Why: luma.gl links one program and then reads back its attributes and uniforms before it builds
// the next, so the driver compiles the map's programs one after another while the page waits. On
// Direct3D drivers each takes a few hundred milliseconds. Programs whose links are all started
// before any is read back compile side by side, and the browser keeps every linked program for
// later contexts with the same settings, so the map's own canvas then links them from that cache.
//
// How: a layer's model takes its pipeline factory from the layer's shader props, and every
// extension's shader props are merged into them. This extension's factory links the two shaders
// the model compiled and hands back a pipeline with no attributes and no bindings, which is enough
// to build the model but not to draw it: layers carrying this extension must be invisible.
import { LayerExtension, type Layer } from '@deck.gl/core';

/** A pipeline the model can be built with but never drawn with. */
interface DeferredPipeline {
  readonly shaderLayout: {
    readonly attributes: readonly never[];
    readonly bindings: readonly never[];
  };
  readonly bufferLayout: readonly never[];
  readonly vs: unknown;
  readonly fs: unknown;
}

/** The WebGL shader behind a luma.gl shader, or null for any other kind of device. */
function webglShaderOf(shader: unknown): WebGLShader | null {
  const handle: unknown =
    typeof shader === 'object' && shader !== null ? Reflect.get(shader, 'handle') : null;
  return handle instanceof WebGLShader ? handle : null;
}

/**
 * Starts links on one WebGL context and never waits for them. Each pair of shaders is linked
 * once, however many models ask.
 */
class DeferredLinker {
  private readonly gl: WebGL2RenderingContext;
  private readonly linked = new Map<WebGLShader, Set<WebGLShader>>();

  constructor(gl: WebGL2RenderingContext) {
    this.gl = gl;
    // Asked for before the first compile: without it the browser compiles only when a result is read.
    gl.getExtension('KHR_parallel_shader_compile');
  }

  /** Starts linking the props' shaders and returns a pipeline that cannot draw. */
  createRenderPipeline(props: { readonly vs?: unknown; readonly fs?: unknown }): DeferredPipeline {
    const vs = webglShaderOf(props.vs);
    const fs = webglShaderOf(props.fs);
    if (vs !== null && fs !== null && this.linked.get(vs)?.has(fs) !== true) {
      const program = this.gl.createProgram();
      this.gl.attachShader(program, vs);
      this.gl.attachShader(program, fs);
      this.gl.linkProgram(program);
      // Sent to the GPU process now; a link left in the command buffer would wait for a read.
      this.gl.flush();
      this.linked.set(vs, (this.linked.get(vs) ?? new Set()).add(fs));
    }
    return {
      shaderLayout: { attributes: [], bindings: [] },
      bufferLayout: [],
      vs: props.vs,
      fs: props.fs,
    };
  }

  /** The model gives its pipeline back when it is destroyed; nothing is held for it. */
  release(): void {}
}

const LINKERS = new WeakMap<WebGL2RenderingContext, DeferredLinker>();

/** The linker for the context `layer` draws into, or null when it is not WebGL 2. */
function linkerOf(layer: Layer): DeferredLinker | null {
  const gl = layer.context.device.handle;
  if (!(gl instanceof WebGL2RenderingContext)) return null;
  const known = LINKERS.get(gl);
  if (known !== undefined) return known;
  const linker = new DeferredLinker(gl);
  LINKERS.set(gl, linker);
  return linker;
}

/** Starts linking each layer's program without waiting; the layers it is on must not be drawn. */
export class PrelinkExtension extends LayerExtension {
  static override extensionName = 'PrelinkExtension';

  override getShaders(this: Layer): Record<string, unknown> {
    const linker = linkerOf(this);
    // The model's attributes are set before any draw, and this pipeline knows none of them.
    return linker === null ? {} : { pipelineFactory: linker, disableWarnings: true };
  }
}
