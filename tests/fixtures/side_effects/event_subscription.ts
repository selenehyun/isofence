// Should detect: event subscriptions at module scope
import { EventEmitter } from 'events';

const emitter = new EventEmitter();
emitter.on('data', (d: any) => console.log(d));
process.addEventListener('uncaughtException', (e: any) => {});
document.addEventListener('click', () => {});

export { emitter };
