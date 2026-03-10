/**
 * TERMINAL - Cypherpunk terminal interface component
 */

export class Terminal {
    constructor(element) {
        this.element = element;
        this.history = [];
        this.maxLines = 500;
    }

    log(message, type = 'default') {
        const line = document.createElement('div');
        line.className = `terminal-line ${type}`;
        
        const timestamp = new Date().toLocaleTimeString('en-US', { 
            hour12: false, 
            hour: '2-digit', 
            minute: '2-digit',
            second: '2-digit'
        });
        
        line.innerHTML = `<span class="timestamp">[${timestamp}]</span> ${this.escapeHtml(message)}`;
        
        this.element.appendChild(line);
        this.history.push({ message, type, timestamp });
        
        // Trim history
        while (this.element.children.length > this.maxLines) {
            this.element.removeChild(this.element.firstChild);
        }
        
        this.scrollToBottom();
    }

    separator(char = '─') {
        const line = document.createElement('div');
        line.className = 'terminal-line separator';
        line.innerHTML = char.repeat(60);
        this.element.appendChild(line);
        this.scrollToBottom();
    }

    // Update the last line instead of adding a new one
    updateLast(message, type = 'default') {
        const lastLine = this.element.lastElementChild;
        if (lastLine && lastLine.classList.contains('updatable')) {
            const timestamp = new Date().toLocaleTimeString('en-US', { 
                hour12: false, 
                hour: '2-digit', 
                minute: '2-digit',
                second: '2-digit'
            });
            lastLine.className = `terminal-line ${type} updatable`;
            lastLine.innerHTML = `<span class="timestamp">[${timestamp}]</span> ${this.escapeHtml(message)}`;
        } else {
            this.logUpdatable(message, type);
        }
        this.scrollToBottom();
    }

    // Log a line that can be updated
    logUpdatable(message, type = 'default') {
        const line = document.createElement('div');
        line.className = `terminal-line ${type} updatable`;
        
        const timestamp = new Date().toLocaleTimeString('en-US', { 
            hour12: false, 
            hour: '2-digit', 
            minute: '2-digit',
            second: '2-digit'
        });
        
        line.innerHTML = `<span class="timestamp">[${timestamp}]</span> ${this.escapeHtml(message)}`;
        this.element.appendChild(line);
        this.scrollToBottom();
    }

    // Clear all updatable lines (clean up after operation)
    clearUpdatable() {
        const updatables = this.element.querySelectorAll('.updatable');
        updatables.forEach(el => el.remove());
    }

    clear() {
        this.element.innerHTML = '';
        this.history = [];
    }

    scrollToBottom() {
        this.element.scrollTop = this.element.scrollHeight;
    }

    escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }

    typewriter(message, type = 'default', speed = 30) {
        return new Promise((resolve) => {
            const line = document.createElement('div');
            line.className = `terminal-line ${type}`;
            
            const timestamp = new Date().toLocaleTimeString('en-US', { 
                hour12: false, 
                hour: '2-digit', 
                minute: '2-digit',
                second: '2-digit'
            });
            
            line.innerHTML = `<span class="timestamp">[${timestamp}]</span> `;
            this.element.appendChild(line);
            
            let i = 0;
            const chars = message.split('');
            
            const type_char = () => {
                if (i < chars.length) {
                    line.innerHTML += this.escapeHtml(chars[i]);
                    i++;
                    this.scrollToBottom();
                    setTimeout(type_char, speed);
                } else {
                    resolve();
                }
            };
            
            type_char();
        });
    }
}


