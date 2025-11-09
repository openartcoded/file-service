new Promise((resolve) => {
            const getDimensions = () => {
                const body = document.body;
                const html = document.documentElement;
                
                const img = document.querySelector('img');
                if (img) {
                    return {
                        width: img.naturalWidth || img.width,
                        height: img.naturalHeight || img.height,
                        x: 0,
                        y: 0
                    };
                }
                
                const width = Math.max(
                    body.scrollWidth, 
                    body.offsetWidth,
                    html.clientWidth,
                    html.scrollWidth,
                    html.offsetWidth
                );
                const height = Math.max(
                    body.scrollHeight,
                    body.offsetHeight, 
                    html.clientHeight,
                    html.scrollHeight,
                    html.offsetHeight
                );
                
                return { width, height, x: 0, y: 0 };
            };
            
            if (document.readyState === 'complete') {
                const img = document.querySelector('img');
                if (img && !img.complete) {
                    img.onload = () => resolve(getDimensions());
                    img.onerror = () => resolve(getDimensions());
                } else {
                    setTimeout(() => resolve(getDimensions()), 100);
                }
            } else {
                window.addEventListener('load', () => {
                    setTimeout(() => resolve(getDimensions()), 100);
                });
            }
        })