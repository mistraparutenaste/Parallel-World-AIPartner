/**
 * Loadable, renderable Live2D character.
 *
 * Promise-based re-implementation of the official sample's LAppModel
 * (CubismWebSamples 5-r.5, Live2D Open Software License) on top of
 * the vendored framework.
 */

import { CubismDefaultParameterId } from '../../vendor/framework/src/cubismdefaultparameterid';
import { CubismModelSettingJson } from '../../vendor/framework/src/cubismmodelsettingjson';
import {
  BreathParameterData,
  CubismBreath,
} from '../../vendor/framework/src/effect/cubismbreath';
import { CubismEyeBlink } from '../../vendor/framework/src/effect/cubismeyeblink';
import {
  CubismLook,
  LookParameterData,
} from '../../vendor/framework/src/effect/cubismlook';
import type { ICubismModelSetting } from '../../vendor/framework/src/icubismmodelsetting';
import type { CubismIdHandle } from '../../vendor/framework/src/id/cubismid';
import { CubismFramework } from '../../vendor/framework/src/live2dcubismframework';
import { CubismMatrix44 } from '../../vendor/framework/src/math/cubismmatrix44';
import { CubismUserModel } from '../../vendor/framework/src/model/cubismusermodel';
import type { ACubismMotion } from '../../vendor/framework/src/motion/acubismmotion';
import type { CubismMotion } from '../../vendor/framework/src/motion/cubismmotion';
import { CubismBreathUpdater } from '../../vendor/framework/src/motion/cubismbreathupdater';
import { CubismExpressionUpdater } from '../../vendor/framework/src/motion/cubismexpressionupdater';
import { CubismEyeBlinkUpdater } from '../../vendor/framework/src/motion/cubismeyeblinkupdater';
import { CubismLipSyncUpdater } from '../../vendor/framework/src/motion/cubismlipsyncupdater';
import { CubismLookUpdater } from '../../vendor/framework/src/motion/cubismlookupdater';
import { CubismPhysicsUpdater } from '../../vendor/framework/src/motion/cubismphysicsupdater';
import { CubismPoseUpdater } from '../../vendor/framework/src/motion/cubismposeupdater';
import { CubismUpdateScheduler } from '../../vendor/framework/src/motion/cubismupdatescheduler';
import { ExternalLipSyncProvider } from '../lip-sync/external-lip-sync-provider';
import type { ModelSource } from '../runtime/cubism-runtime';
import { resolveIdleGroup } from './idle-group';

const PRIORITY_IDLE = 1;
const PRIORITY_NORMAL = 2;

type GL = WebGLRenderingContext | WebGL2RenderingContext;

async function fetchArrayBuffer(url: string): Promise<ArrayBuffer> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`failed to fetch ${url}: ${response.status}`);
  }
  return response.arrayBuffer();
}

export class CharacterModel extends CubismUserModel {
  #gl: GL;
  #canvas: HTMLCanvasElement;
  #shaderPath: string;
  #source: ModelSource | null = null;
  #setting: ICubismModelSetting | null = null;
  #motions = new Map<string, CubismMotion>();
  #expressionMotions = new Map<string, ACubismMotion>();
  #expressionNames: string[] = [];
  #motionGroupCounts = new Map<string, number>();
  #idleGroup: string | null = null;
  #eyeBlinkIds: CubismIdHandle[] = [];
  #lipSyncIds: CubismIdHandle[] = [];
  #lipSyncProvider = new ExternalLipSyncProvider();
  #textures: WebGLTexture[] = [];
  #scheduler = new CubismUpdateScheduler();
  #motionUpdated = false;
  #ready = false;

  constructor(gl: GL, canvas: HTMLCanvasElement, shaderPath: string) {
    super();
    this.#gl = gl;
    this.#canvas = canvas;
    this.#shaderPath = shaderPath;
  }

  get expressionNames(): readonly string[] {
    return this.#expressionNames;
  }

  get motionGroupCounts(): ReadonlyMap<string, number> {
    return this.#motionGroupCounts;
  }

  /** Loads every resource referenced by the model3.json source. */
  async load(source: ModelSource): Promise<void> {
    this.#source = source;
    const settingBuffer = await fetchArrayBuffer(source.modelUrl);
    const setting: ICubismModelSetting = new CubismModelSettingJson(
      settingBuffer,
      settingBuffer.byteLength,
    );
    this.#setting = setting;

    await this.#loadMoc(setting);
    await this.#loadExpressions(setting);
    await this.#loadPhysics(setting);
    await this.#loadPose(setting);
    this.#setupEffects(setting);
    this.#setupLayout(setting);
    await this.#loadMotions(setting);
    await this.#setupRenderer(setting);
    await this.#loadTextures(setting);

    this._model.saveParameters();
    this.#ready = true;
  }

  /** Advances motions, effects and physics by the given delta. */
  updateFrame(deltaSeconds: number): void {
    if (!this.#ready) {
      return;
    }
    this._model.loadParameters();
    this.#motionUpdated = false;
    if (this._motionManager.isFinished()) {
      if (this.#idleGroup !== null) {
        this.startMotionIn(this.#idleGroup, undefined, PRIORITY_IDLE);
      }
    } else {
      this.#motionUpdated = this._motionManager.updateMotion(
        this._model,
        deltaSeconds,
      );
    }
    this._model.saveParameters();

    this.#scheduler.onLateUpdate(this._model, deltaSeconds);
    this._model.update();
  }

  /** Draws the model with the given projection matrix. */
  draw(projection: CubismMatrix44): void {
    if (!this.#ready || this._model == null) {
      return;
    }
    const matrix = new CubismMatrix44();
    matrix.setMatrix(projection.getArray());
    matrix.multiplyByMatrix(this._modelMatrix);
    this.getRenderer().setMvpMatrix(matrix);
    this.getRenderer().setRenderState(
      null as unknown as WebGLFramebuffer,
      [0, 0, this.#canvas.width, this.#canvas.height],
    );
    this.getRenderer().drawModel(this.#shaderPath);
  }

  /** Applies a named expression. Returns false for unknown names. */
  setExpressionByName(name: string): boolean {
    if (!this.#expressionNames.includes(name)) {
      return false;
    }
    const motion = this.#expressionMotions.get(name);
    if (motion == null) {
      return false;
    }
    this._expressionManager.startMotion(motion, false);
    return true;
  }

  /**
   * Starts a motion from a group (random index when omitted).
   * Returns false for unknown groups or indices.
   */
  startMotionIn(
    group: string,
    index?: number,
    priority: number = PRIORITY_NORMAL,
  ): boolean {
    const count = this.#motionGroupCounts.get(group);
    if (count === undefined || count === 0) {
      return false;
    }
    const no = index ?? Math.floor(Math.random() * count);
    const motion = this.#motions.get(`${group}_${no}`);
    if (motion === undefined) {
      return false;
    }
    if (!this._motionManager.reserveMotion(priority)) {
      return false;
    }
    this._motionManager.startMotionPriority(motion, false, priority);
    return true;
  }

  /** Stops the active gesture and starts the first stable idle motion. */
  startIdleMotion(): boolean {
    if (this.#idleGroup === null) {
      return false;
    }
    this._motionManager.stopAllMotions();
    // Cubism clears the current priority only while updating a finished
    // queue. Do that synchronously so the lower-priority idle motion can
    // start now instead of being rejected behind the stopped speech motion.
    this._motionManager.updateMotion(this._model, 0);
    return this.startMotionIn(this.#idleGroup, 0, PRIORITY_IDLE);
  }

  /**
   * Sets the mouth-open value (0..1) computed from the playing audio.
   * Applied to the model's LipSync parameters every frame.
   */
  setLipSyncValue(value: number): void {
    this.#lipSyncProvider.setValue(value);
  }

  /** Releases GL textures and framework resources. */
  override release(): void {
    for (const texture of this.#textures) {
      this.#gl.deleteTexture(texture);
    }
    this.#textures = [];
    this.#scheduler.release();
    super.release();
  }

  #resolveResource(relativePath: string): string {
    if (this.#source == null) {
      throw new Error('model source is not set');
    }
    return this.#source.resolveResource(relativePath);
  }

  async #loadMoc(setting: ICubismModelSetting): Promise<void> {
    const fileName = setting.getModelFileName();
    if (fileName === '') {
      throw new Error('model3.json does not reference a moc3 file');
    }
    const buffer = await fetchArrayBuffer(
      this.#resolveResource(fileName),
    );
    this.loadModel(buffer, this._mocConsistency);
  }

  async #loadExpressions(setting: ICubismModelSetting): Promise<void> {
    const count = setting.getExpressionCount();
    for (let i = 0; i < count; i++) {
      const name = setting.getExpressionName(i);
      const file = setting.getExpressionFileName(i);
      const buffer = await fetchArrayBuffer(
        this.#resolveResource(file),
      );
      const motion = this.loadExpression(buffer, buffer.byteLength, name);
      this.#expressionMotions.set(name, motion);
      this.#expressionNames.push(name);
    }
    if (count > 0 && this._expressionManager != null) {
      this.#scheduler.addUpdatableList(
        new CubismExpressionUpdater(this._expressionManager),
      );
    }
  }

  async #loadPhysics(setting: ICubismModelSetting): Promise<void> {
    const fileName = setting.getPhysicsFileName();
    if (fileName === '') {
      return;
    }
    const buffer = await fetchArrayBuffer(
      this.#resolveResource(fileName),
    );
    this.loadPhysics(buffer, buffer.byteLength);
    if (this._physics) {
      this.#scheduler.addUpdatableList(new CubismPhysicsUpdater(this._physics));
    }
  }

  async #loadPose(setting: ICubismModelSetting): Promise<void> {
    const fileName = setting.getPoseFileName();
    if (fileName === '') {
      return;
    }
    const buffer = await fetchArrayBuffer(
      this.#resolveResource(fileName),
    );
    this.loadPose(buffer, buffer.byteLength);
    if (this._pose) {
      this.#scheduler.addUpdatableList(new CubismPoseUpdater(this._pose));
    }
  }

  #setupEffects(setting: ICubismModelSetting): void {
    const idManager = CubismFramework.getIdManager();

    if (setting.getEyeBlinkParameterCount() > 0) {
      this._eyeBlink = CubismEyeBlink.create(setting);
      this.#scheduler.addUpdatableList(
        new CubismEyeBlinkUpdater(() => this.#motionUpdated, this._eyeBlink),
      );
    }
    for (let i = 0; i < setting.getEyeBlinkParameterCount(); ++i) {
      this.#eyeBlinkIds.push(setting.getEyeBlinkParameterId(i));
    }
    for (let i = 0; i < setting.getLipSyncParameterCount(); ++i) {
      this.#lipSyncIds.push(setting.getLipSyncParameterId(i));
    }
    // Models without a LipSync group still get audio lip sync through
    // the standard mouth parameter.
    if (this.#lipSyncIds.length === 0) {
      this.#lipSyncIds.push(
        idManager.getId(CubismDefaultParameterId.ParamMouthOpenY),
      );
    }
    this.#scheduler.addUpdatableList(
      new CubismLipSyncUpdater(this.#lipSyncIds, this.#lipSyncProvider),
    );

    const angleX = idManager.getId(CubismDefaultParameterId.ParamAngleX);
    const angleY = idManager.getId(CubismDefaultParameterId.ParamAngleY);
    const angleZ = idManager.getId(CubismDefaultParameterId.ParamAngleZ);
    const bodyAngleX = idManager.getId(
      CubismDefaultParameterId.ParamBodyAngleX,
    );

    this._breath = CubismBreath.create();
    this._breath.setParameters([
      new BreathParameterData(angleX, 0.0, 15.0, 6.5345, 0.5),
      new BreathParameterData(angleY, 0.0, 8.0, 3.5345, 0.5),
      new BreathParameterData(angleZ, 0.0, 10.0, 5.5345, 0.5),
      new BreathParameterData(bodyAngleX, 0.0, 4.0, 15.5345, 0.5),
      new BreathParameterData(
        idManager.getId(CubismDefaultParameterId.ParamBreath),
        0.5,
        0.5,
        3.2345,
        1,
      ),
    ]);
    this.#scheduler.addUpdatableList(new CubismBreathUpdater(this._breath));

    const look = CubismLook.create();
    look.setParameters([
      new LookParameterData(angleX, 30.0, 0.0, 0.0),
      new LookParameterData(angleY, 0.0, 30.0, 0.0),
      new LookParameterData(angleZ, 0.0, 0.0, -30.0),
      new LookParameterData(bodyAngleX, 10.0, 0.0, 0.0),
      new LookParameterData(
        idManager.getId(CubismDefaultParameterId.ParamEyeBallX),
        1.0,
        0.0,
        0.0,
      ),
      new LookParameterData(
        idManager.getId(CubismDefaultParameterId.ParamEyeBallY),
        0.0,
        1.0,
        0.0,
      ),
    ]);
    this.#scheduler.addUpdatableList(
      new CubismLookUpdater(look, this._dragManager),
    );

    this.#scheduler.sortUpdatableList();
  }

  #setupLayout(setting: ICubismModelSetting): void {
    const layout = new Map<string, number>();
    setting.getLayoutMap(layout);
    this._modelMatrix.setupFromLayout(layout);
  }

  async #loadMotions(setting: ICubismModelSetting): Promise<void> {
    const groupCount = setting.getMotionGroupCount();
    const groups: string[] = [];
    for (let i = 0; i < groupCount; i++) {
      groups.push(setting.getMotionGroupName(i));
    }
    for (const group of groups) {
      const count = setting.getMotionCount(group);
      this.#motionGroupCounts.set(group, count);
      for (let no = 0; no < count; no++) {
        const file = setting.getMotionFileName(group, no);
        const buffer = await fetchArrayBuffer(
          this.#resolveResource(file),
        );
        const motion = this.loadMotion(
          buffer,
          buffer.byteLength,
          `${group}_${no}`,
          null,
          null,
          setting,
          group,
          no,
          this._motionConsistency,
        );
        if (motion != null) {
          motion.setEffectIds(this.#eyeBlinkIds, this.#lipSyncIds);
          this.#motions.set(`${group}_${no}`, motion);
        }
      }
    }
    this.#idleGroup = resolveIdleGroup(groups);
    this._motionManager.stopAllMotions();
  }

  async #setupRenderer(_setting: ICubismModelSetting): Promise<void> {
    this.createRenderer(this.#canvas.width, this.#canvas.height);
    this.getRenderer().startUp(this.#gl);
    this.getRenderer().setIsPremultipliedAlpha(true);
    await this.getRenderer().loadShaders(this.#shaderPath);
  }

  async #loadTextures(setting: ICubismModelSetting): Promise<void> {
    const gl = this.#gl;
    const count = setting.getTextureCount();
    for (let i = 0; i < count; i++) {
      const file = setting.getTextureFileName(i);
      if (file === '') {
        continue;
      }
      const response = await fetch(this.#resolveResource(file));
      if (!response.ok) {
        throw new Error(`failed to fetch texture ${file}: ${response.status}`);
      }
      const bitmap = await createImageBitmap(await response.blob(), {
        premultiplyAlpha: 'premultiply',
      });
      const texture = gl.createTexture();
      if (texture == null) {
        throw new Error('failed to create WebGL texture');
      }
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(
        gl.TEXTURE_2D,
        gl.TEXTURE_MIN_FILTER,
        gl.LINEAR_MIPMAP_LINEAR,
      );
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, bitmap);
      gl.generateMipmap(gl.TEXTURE_2D);
      gl.bindTexture(gl.TEXTURE_2D, null);
      bitmap.close();
      this.getRenderer().bindTexture(i, texture);
      this.#textures.push(texture);
    }
  }
}
