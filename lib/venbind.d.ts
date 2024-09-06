export class Venbind {
    startKeybinds(window_id: BigInt | null, display_id: BigInt | null, callback: (x: number) => void): Promise<void>;
    registerKeybind(keybind: string, keybindId: number): void;
    unregisterKeybind(keybindId: number): void;
}
